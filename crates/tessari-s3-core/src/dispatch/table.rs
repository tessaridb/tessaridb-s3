//! The dispatch decision table: a request to exactly one catalog operation, or a refusal.

use super::catalog::CATALOG;
use super::error::DispatchError;
use super::spec::{Method, OperationSpec, Target};

/// What dispatch looks at: the method, what the path addresses, the decoded query and the header names.
#[derive(Debug, Clone, Copy)]
pub struct DispatchRequest<'a> {
    /// The method.
    pub method: Method,
    /// What the path addresses; `Named` carries the literal path for fixed-path operations.
    pub target: Target,
    /// Decoded query parameters: name, and the value when one was given (`?acl` has none).
    pub query: &'a [(&'a str, Option<&'a str>)],
    /// Header names, any case.
    pub header_names: &'a [&'a str],
}

/// Query parameters every request may carry whatever its operation: SigV4 presign parameters, and `x-id`, the
/// operation name some AWS SDKs append to the query.
const UNIVERSAL_QUERY: [&str; 8] = [
    "X-Amz-Algorithm",
    "X-Amz-Credential",
    "X-Amz-Date",
    "X-Amz-Expires",
    "X-Amz-SignedHeaders",
    "X-Amz-Signature",
    "X-Amz-Security-Token",
    "x-id",
];

/// Resolves a request to its operation.
///
/// A row matches when the method and target are its own, every subresource it names is present (with its fixed
/// value, when it has one), and no other catalog subresource is present. Of the rows that match, the one whose
/// required parameters and headers are all present and that requires the most wins. Every query parameter must
/// then be one the winner models.
///
/// # Errors
/// A [`DispatchError`]; there is no default operation.
pub fn dispatch(request: &DispatchRequest<'_>) -> Result<&'static OperationSpec, DispatchError> {
    let on_target: Vec<&'static OperationSpec> = CATALOG
        .iter()
        .filter(|spec| {
            spec.dispatchable && spec.method == request.method && spec.target == request.target
        })
        .collect();
    if on_target.is_empty() {
        return Err(DispatchError::MethodNotAllowed);
    }
    let shaped: Vec<&'static OperationSpec> = on_target
        .iter()
        .copied()
        .filter(|spec| shape_matches(spec, request))
        .collect();
    if shaped.is_empty() {
        return Err(no_shape_match(&on_target, request));
    }
    let mut chosen: Option<&'static OperationSpec> = None;
    for spec in shaped
        .iter()
        .copied()
        .filter(|spec| requirements_present(spec, request))
    {
        match chosen {
            Some(best) if best.specificity() > spec.specificity() => {}
            Some(best) if best.specificity() == spec.specificity() => {
                return Err(DispatchError::Ambiguous {
                    first: best.name,
                    second: spec.name,
                });
            }
            _ => chosen = Some(spec),
        }
    }
    let Some(chosen) = chosen else {
        return Err(missing_requirement(&shaped, request));
    };
    if let Some((name, _)) = request
        .query
        .iter()
        .find(|(name, _)| !models_parameter(chosen, name))
    {
        return Err(DispatchError::UnknownQueryParameter {
            name: (*name).to_owned(),
        });
    }
    Ok(chosen)
}

/// Every subresource of `spec` present with its fixed value, and no other catalog subresource present.
fn shape_matches(spec: &OperationSpec, request: &DispatchRequest<'_>) -> bool {
    let own_present = spec.discriminators.iter().all(|(name, fixed)| {
        request
            .query
            .iter()
            .any(|(q, value)| q == name && fixed.is_none_or(|fixed| *value == Some(fixed)))
    });
    let no_foreign = request.query.iter().all(|(q, _)| {
        !is_catalog_subresource(q) || spec.discriminators.iter().any(|(name, _)| name == q)
    });
    own_present && no_foreign
}

/// Every required query parameter and header of `spec` present.
fn requirements_present(spec: &OperationSpec, request: &DispatchRequest<'_>) -> bool {
    spec.required_query
        .iter()
        .all(|name| request.query.iter().any(|(q, _)| q == name))
        && spec
            .required_headers
            .iter()
            .all(|name| has_header(request, name))
}

/// The refusal when the method and target have rows but no row's subresources fit: a subresource this method and
/// target never take is a method the resource does not allow; anything else is a parameter nobody models.
fn no_shape_match(
    on_target: &[&'static OperationSpec],
    request: &DispatchRequest<'_>,
) -> DispatchError {
    let foreign = request.query.iter().find(|(q, _)| {
        is_catalog_subresource(q)
            && !on_target
                .iter()
                .any(|spec| spec.discriminators.iter().any(|(name, _)| name == q))
    });
    if foreign.is_some() {
        return DispatchError::MethodNotAllowed;
    }
    let offending = request
        .query
        .iter()
        .find(|(q, _)| is_catalog_subresource(q))
        .map_or("", |(q, _)| *q);
    DispatchError::UnknownQueryParameter {
        name: offending.to_owned(),
    }
}

/// The refusal when subresources select rows whose required parameters or headers are absent.
fn missing_requirement(
    shaped: &[&'static OperationSpec],
    request: &DispatchRequest<'_>,
) -> DispatchError {
    let Some(spec) = shaped.iter().max_by_key(|spec| spec.discriminators.len()) else {
        return DispatchError::MethodNotAllowed;
    };
    let missing_query = spec
        .required_query
        .iter()
        .find(|name| !request.query.iter().any(|(q, _)| q == *name));
    let missing_header = spec
        .required_headers
        .iter()
        .find(|name| !has_header(request, name));
    match missing_query.or(missing_header) {
        Some(missing) => DispatchError::MissingParameter {
            operation: spec.name,
            missing,
        },
        None => DispatchError::MethodNotAllowed,
    }
}

/// Whether `name` is a parameter `spec` models, one of its subresources, or a universal parameter.
fn models_parameter(spec: &OperationSpec, name: &str) -> bool {
    spec.query.contains(&name)
        || spec
            .discriminators
            .iter()
            .any(|(subresource, _)| *subresource == name)
        || UNIVERSAL_QUERY.contains(&name)
}

/// Whether some catalog operation names `name` as a subresource of its URI.
fn is_catalog_subresource(name: &str) -> bool {
    CATALOG.iter().any(|spec| {
        spec.discriminators
            .iter()
            .any(|(subresource, _)| *subresource == name)
    })
}

/// Whether the request carries `name`, compared case-insensitively.
fn has_header(request: &DispatchRequest<'_>, name: &str) -> bool {
    request
        .header_names
        .iter()
        .any(|header| header.eq_ignore_ascii_case(name))
}
