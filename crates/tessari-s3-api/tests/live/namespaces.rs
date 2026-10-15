//! A namespace of its own for every live test, removed when the test ends — returned or failed — so a run leaves the
//! test node with the namespaces it found. Names are `t_<pid>_<uuid>`; the first name a process makes also removes a
//! crashed run's leftovers: namespaces of a process no longer running, the older `t_<uuid>` shape, and accounts left
//! with no namespace. Nothing this harness did not name is touched.

use std::sync::Once;

use tessari_s3_infrastructure::tessaridb::{
    Answer, MetaError, MetaPool, MetaSettings, RefusalClass, Value,
};

use crate::SCHEMA_TURN;

/// How often one teardown statement is tried while the catalog is contended.
const ATTEMPTS: usize = 5;

static SWEPT: Once = Once::new();

/// A fresh namespace name for one test; [`removed_at_end`] takes it away again.
pub(crate) fn name() -> String {
    format!("t_{}_{}", std::process::id(), uuid::Uuid::new_v4().simple())
}

/// Removes `settings.namespace` with everything in it when the calling test ends, returned or failed. `settings` reach
/// the test node as its owner; must be called inside the test's runtime.
pub(crate) fn removed_at_end(settings: &MetaSettings) {
    SWEPT.call_once(|| on_own_runtime(sweep(settings.clone())));
    let removal = Removal {
        settings: settings.clone(),
        namespace: settings.namespace.clone(),
    };
    // The runtime drops this task — and the removal with it — when the test's body has returned or unwound.
    tokio::spawn(async move {
        let _removal = removal;
        std::future::pending::<()>().await;
    });
}

/// Removes `namespace` when dropped.
struct Removal {
    settings: MetaSettings,
    namespace: String,
}

impl Drop for Removal {
    fn drop(&mut self) {
        let settings = self.settings.clone();
        let namespace = std::mem::take(&mut self.namespace);
        on_own_runtime(async move {
            if let Err(error) = remove(&owner(settings), &namespace).await {
                report(format_args!(
                    "the live suite could not remove namespace {namespace}: {error}"
                ));
            }
        });
    }
}

/// Runs `work` to completion on a thread and runtime of its own: the test's runtime is shutting down, or busy.
fn on_own_runtime(work: impl Future<Output = ()> + Send + 'static) {
    let finished = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the namespace work")
            .block_on(work);
    })
    .join();
    if finished.is_err() {
        report(format_args!("the live suite's namespace work panicked"));
    }
}

/// Says what went wrong on the process's own stderr: the test harness captures `eprintln!`, even from this thread, and
/// shows it only for a failed test.
fn report(message: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{message}");
}

fn owner(settings: MetaSettings) -> MetaPool {
    MetaPool::new(settings).expect("the test node's pool")
}

/// Removes a crashed run's leftovers from the test node.
pub(crate) async fn sweep(settings: MetaSettings) {
    let pool = owner(settings);
    let (namespaces, users) = match catalog(&pool).await {
        Ok(found) => found,
        Err(error) => {
            report(format_args!(
                "the live suite could not read the test node's namespaces: {error}"
            ));
            return;
        }
    };
    for namespace in namespaces.iter().filter(|name| left_over(name)) {
        if let Err(error) = remove(&pool, namespace).await {
            report(format_args!(
                "the live suite could not remove leftover namespace {namespace}: {error}"
            ));
        }
    }
    let orphans = users
        .iter()
        .filter(|(user, namespace)| user.starts_with("svc_t_") && namespace.is_none());
    for (user, _) in orphans {
        if let Err(error) = turn(&pool, &format!("DROP USER IF EXISTS {user};")).await {
            report(format_args!(
                "the live suite could not remove leftover account {user}: {error}"
            ));
        }
    }
}

/// Whether `namespace` is a name this harness makes whose process is gone.
fn left_over(namespace: &str) -> bool {
    let Some(rest) = namespace.strip_prefix("t_") else {
        return false;
    };
    let hex = |text: &str| text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if hex(rest) {
        return true;
    }
    match rest.split_once('_') {
        Some((pid, id)) if hex(id) && pid.parse::<u32>().is_ok() => !running(pid),
        _ => false,
    }
}

/// Whether a process `pid` runs on this machine.
fn running(pid: &str) -> bool {
    std::process::Command::new("ps")
        .args(["-p", pid])
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The node's namespaces, and its users with the namespace each is confined to.
async fn catalog(
    pool: &MetaPool,
) -> Result<(Vec<String>, Vec<(String, Option<String>)>), MetaError> {
    let answers = pool.run_unscoped("INFO FOR STORE; INFO FOR USERS;").await?;
    let namespaces = list(answers.first(), "namespaces")
        .iter()
        .filter_map(text)
        .collect();
    let users = list(answers.get(1), "users")
        .iter()
        .filter_map(|row| match row {
            Value::Object(row) => Some((
                row.get("user").and_then(text)?,
                row.get("namespace").and_then(text),
            )),
            _ => None,
        })
        .collect();
    Ok((namespaces, users))
}

/// Removes `namespace`, the accounts confined to it and everything in it; a namespace that does not exist is done.
/// The engine drops no further than one name, so the order is accounts, tables, databases, the namespace.
async fn remove(pool: &MetaPool, namespace: &str) -> Result<(), MetaError> {
    let (namespaces, users) = catalog(pool).await?;
    for (user, _) in users
        .iter()
        .filter(|(_, confined)| confined.as_deref() == Some(namespace))
    {
        turn(pool, &format!("DROP USER IF EXISTS {user};")).await?;
    }
    if !namespaces.iter().any(|name| name == namespace) {
        return Ok(());
    }
    let info = pool
        .run_unscoped(&format!("USE NAMESPACE {namespace}; INFO FOR NAMESPACE;"))
        .await?;
    let databases: Vec<String> = list(info.get(1), "databases")
        .iter()
        .filter_map(text)
        .collect();
    for database in databases {
        remove_database(pool, namespace, &database).await?;
    }
    turn(pool, &format!("DROP NAMESPACE {namespace};")).await
}

/// Drops every table of `database` and the database in one transaction, so a teardown cut short leaves nothing half
/// removed; read again and retried while it is refused, so a table that appeared meanwhile goes too.
async fn remove_database(
    pool: &MetaPool,
    namespace: &str,
    database: &str,
) -> Result<(), MetaError> {
    let scope = format!("USE NAMESPACE {namespace}; USE DATABASE {database};");
    for _ in 1..ATTEMPTS {
        match drop_all(pool, &scope, database).await {
            Err(MetaError::Refused { message, .. }) => report(format_args!(
                "the live suite retries dropping {namespace}.{database}: {message}"
            )),
            outcome => return outcome,
        }
    }
    drop_all(pool, &scope, database).await
}

/// One attempt of [`remove_database`]: the tables `scope` holds now and `database`, in one transaction.
async fn drop_all(pool: &MetaPool, scope: &str, database: &str) -> Result<(), MetaError> {
    let info = pool
        .run_unscoped(&format!("{scope} INFO FOR DATABASE;"))
        .await?;
    let drops: String = list(info.get(2), "tables")
        .iter()
        .filter_map(text)
        .map(|table| format!(" DROP TABLE {table};"))
        .collect();
    turn(
        pool,
        &format!("{scope} BEGIN;{drops} DROP DATABASE {database}; COMMIT;"),
    )
    .await
}

/// Runs a catalog change in the schema turn the tests' own definitions take (Q-S3-2), again while it is contended.
async fn turn(pool: &MetaPool, script: &str) -> Result<(), MetaError> {
    let _turn = SCHEMA_TURN.lock().await;
    for _ in 1..ATTEMPTS {
        match pool.run_unscoped(script).await {
            Err(MetaError::Refused {
                class: Some(RefusalClass::Retry),
                ..
            }) => {}
            outcome => return outcome.map(drop),
        }
    }
    pool.run_unscoped(script).await.map(drop)
}

/// The array `field` of a value answer; empty when the answer has another shape.
fn list<'a>(answer: Option<&'a Answer>, field: &str) -> &'a [Value] {
    match answer {
        Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) => match info.get(field) {
            Some(Value::Array(items)) => items,
            _ => &[],
        },
        _ => &[],
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}
