//! The shape of a catalog row: what a request must carry to be one operation.

use super::catalog::Operation;

/// The HTTP methods the S3 model uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `HEAD`
    Head,
    /// `PUT`
    Put,
    /// `POST`
    Post,
    /// `DELETE`
    Delete,
}

impl Method {
    /// The method for a request-line token; `None` for one the model never uses.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "GET" => Some(Self::Get),
            "HEAD" => Some(Self::Head),
            "PUT" => Some(Self::Put),
            "POST" => Some(Self::Post),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// What the request path addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// `/` — the service.
    Service,
    /// `/{Bucket}`.
    Bucket,
    /// `/{Bucket}/{Key+}`.
    Object,
    /// A fixed path outside the bucket namespace, e.g. `/WriteGetObjectResponse`.
    Named(&'static str),
}

/// One operation of the model and what selects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationSpec {
    /// The operation.
    pub operation: Operation,
    /// Its model name, e.g. `PutObject`.
    pub name: &'static str,
    /// Its method.
    pub method: Method,
    /// What its path addresses.
    pub target: Target,
    /// Subresources from its URI (`?acl`, `?list-type=2`): present, with the value when one is fixed.
    pub discriminators: &'static [(&'static str, Option<&'static str>)],
    /// Query parameters it requires (`uploadId`, `partNumber`).
    pub required_query: &'static [&'static str],
    /// Headers it requires (`x-amz-copy-source`), lowercase.
    pub required_headers: &'static [&'static str],
    /// Every query parameter it models.
    pub query: &'static [&'static str],
    /// `false` when another operation with the same wire shape answers it (a deprecated twin, or an operation
    /// that differs only by endpoint type).
    pub dispatchable: bool,
}

impl OperationSpec {
    /// How specific the row is: the more it requires, the earlier it wins over a row it overlaps.
    #[must_use]
    pub const fn specificity(&self) -> usize {
        self.discriminators
            .len()
            .saturating_add(self.required_query.len())
            .saturating_add(self.required_headers.len())
    }
}
