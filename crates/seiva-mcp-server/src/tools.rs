//! The tools of a set of facades: names, schemas, permissions and dispatch.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use seiva::contract::{FacadeDescriptor, Param, ToolDescriptor, ToolEffect};
use seiva_contract::Problem;
use seiva_http::Ctx;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

/// Why a tool call did not produce a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure {
    message: String,
}

impl ToolFailure {
    /// A failure with this message (no argument values in it).
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ToolFailure {}

/// What `#[seiva::tools]` hands over: the future of one call.
pub type ToolFuture = Pin<Box<dyn Future<Output = Result<Value, ToolFailure>> + Send>>;

type Dispatch = dyn Fn(Ctx, String, Value) -> ToolFuture + Send + Sync;

/// The tools of one facade, as `#[seiva::tools]` generates them.
#[derive(Clone)]
pub struct FacadeTools {
    descriptor: FacadeDescriptor,
    tools: &'static [ToolDescriptor],
    dispatch: Arc<Dispatch>,
}

impl FacadeTools {
    /// The facade's descriptor, its `#[tool]` methods and the dispatcher.
    pub fn new<F>(
        descriptor: FacadeDescriptor,
        tools: &'static [ToolDescriptor],
        dispatch: F,
    ) -> Self
    where
        F: Fn(Ctx, String, Value) -> ToolFuture + Send + Sync + 'static,
    {
        Self {
            descriptor,
            tools,
            dispatch: Arc::new(dispatch),
        }
    }

    /// The facade's descriptor.
    pub fn descriptor(&self) -> &FacadeDescriptor {
        &self.descriptor
    }
}

/// One tool, as listed to a client.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    /// `<facade>_<method>` in snake case: `payments_refund`.
    pub name: String,
    /// The facade.
    pub facade: &'static str,
    /// The facade method.
    pub method: &'static str,
    /// What it does, for the model.
    pub description: &'static str,
    /// Read or write.
    pub effect: ToolEffect,
    /// The permission a caller must hold (`""` for a public read tool).
    pub requires: &'static str,
    /// The JSON Schema of its arguments.
    pub input_schema: Value,
}

/// Every tool of a service, by name.
#[derive(Clone, Default)]
pub struct ToolSet {
    tools: BTreeMap<String, (Tool, Arc<Dispatch>)>,
}

/// Why a tool set could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSetError(pub String);

impl fmt::Display for ToolSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot build the tool set: {}", self.0)
    }
}

impl std::error::Error for ToolSetError {}

impl ToolSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a facade's tools. Two tools with the same name are refused, as is a
    /// `#[tool]` the descriptor does not know.
    pub fn with(mut self, facade: FacadeTools) -> Result<Self, ToolSetError> {
        for t in facade.tools {
            let method = facade.descriptor.method(t.method).ok_or_else(|| {
                ToolSetError(format!(
                    "`{}` is not a method of `{}`",
                    t.method, facade.descriptor.name
                ))
            })?;
            let name = format!("{}_{}", snake(facade.descriptor.name), t.method);
            if name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(ToolSetError(format!(
                    "tool name `{name}` is not a valid MCP tool name"
                )));
            }
            if self.tools.contains_key(&name) {
                return Err(ToolSetError(format!("tool `{name}` is defined twice")));
            }
            let tool = Tool {
                name: name.clone(),
                facade: facade.descriptor.name,
                method: t.method,
                description: t.description,
                effect: t.effect,
                requires: method.requires,
                input_schema: input_schema(method.params),
            };
            self.tools.insert(name, (tool, facade.dispatch.clone()));
        }
        Ok(self)
    }

    /// Every tool, by name.
    pub fn tools(&self) -> impl Iterator<Item = &Tool> {
        self.tools.values().map(|(t, _)| t)
    }

    /// The tool called `name`.
    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name).map(|(t, _)| t)
    }

    /// How many tools there are.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Call `name` in-process as `principal` — for an agent in the same process
    /// (`seiva-agent`). The tool's `#[requires]` is checked here: a tool the
    /// principal may not use fails as unknown. Approving a write is the caller's
    /// job (the agent's approver); nothing here asks a person.
    pub async fn invoke(
        &self,
        name: &str,
        principal: &seiva_identity::Principal,
        args: Value,
    ) -> Result<Value, ToolFailure> {
        let permitted = self
            .get(name)
            .is_some_and(|t| t.requires.is_empty() || principal.has(t.requires));
        if !permitted {
            return Err(ToolFailure::new(format!("unknown tool `{name}`")));
        }
        match self.call(name, Ctx::new(principal.clone()), args) {
            Some(future) => future.await,
            None => Err(ToolFailure::new(format!("unknown tool `{name}`"))),
        }
    }

    /// Call `name` with `args` as the caller of `ctx`. The caller's permission is
    /// the server's to check before this.
    pub(crate) fn call(&self, name: &str, ctx: Ctx, args: Value) -> Option<ToolFuture> {
        let (tool, dispatch) = self.tools.get(name)?;
        Some(dispatch(ctx, tool.method.to_owned(), args))
    }
}

/// `PaymentMethods` → `payment_methods`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether a parameter type is the request context (`Ctx`, `&Ctx`, a path to it).
fn is_ctx(ty: &str) -> bool {
    let last = ty
        .trim_start_matches('&')
        .trim()
        .rsplit("::")
        .next()
        .unwrap_or_default()
        .trim();
    last == "Ctx"
}

/// The tool's one wire parameter (the `&Ctx` excluded).
fn primary(params: &[Param]) -> Option<&Param> {
    params.iter().find(|p| !is_ctx(p.ty))
}

/// The schema of a named type: a `WireSchema` registration, or the scalar the
/// name says.
fn type_schema(ty: &str) -> Value {
    if let Some(schema) = seiva::schema::registered_schema(ty) {
        return schema;
    }
    match ty.trim_start_matches('&').trim() {
        "String" | "str" => json!({"type": "string"}),
        "bool" => json!({"type": "boolean"}),
        "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => {
            json!({"type": "integer"})
        }
        "f32" | "f64" => json!({"type": "number"}),
        _ => json!({}),
    }
}

/// Whether the parameter's own object schema is the tool's input (its fields are
/// the arguments) rather than one argument named after the parameter.
fn flattened(param: &Param) -> bool {
    seiva::schema::registered_schema(param.ty)
        .is_some_and(|s| s.get("type").and_then(Value::as_str) == Some("object"))
}

/// A tool's input schema: the parameter DTO's object schema when it has one,
/// otherwise an object with one property named after the parameter.
pub(crate) fn input_schema(params: &[Param]) -> Value {
    match primary(params) {
        None => json!({"type": "object", "properties": {}, "additionalProperties": false}),
        Some(p) if flattened(p) => type_schema(p.ty),
        Some(p) => json!({
            "type": "object",
            "properties": {p.name: type_schema(p.ty)},
            "required": [p.name],
            "additionalProperties": false,
        }),
    }
}

/// Decode a tool's arguments into `method`'s parameter, the same way
/// [`input_schema`] described them.
pub(crate) fn from_args<T: DeserializeOwned>(
    args: &Value,
    descriptor: &FacadeDescriptor,
    method: &str,
) -> Result<T, ToolFailure> {
    let param = descriptor.method(method).and_then(|m| primary(m.params));
    let value = match param {
        Some(p) if flattened(p) => args.clone(),
        Some(p) => args.get(p.name).cloned().unwrap_or(Value::Null),
        None => args.clone(),
    };
    serde_json::from_value(value)
        .map_err(|e| ToolFailure::new(format!("the arguments do not match the tool's schema: {e}")))
}

pub(crate) fn to_value<T: Serialize>(value: &T) -> Result<Value, ToolFailure> {
    serde_json::to_value(value)
        .map_err(|e| ToolFailure::new(format!("the result cannot be encoded: {e}")))
}

pub(crate) fn failed<E: Into<Problem>>(error: E) -> ToolFailure {
    ToolFailure::new(error.into().to_string())
}

pub(crate) fn unknown(method: &str) -> ToolFailure {
    ToolFailure::new(format!("no tool dispatch for method `{method}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_snake_case_and_ctx_is_not_an_argument() {
        assert_eq!(snake("PaymentMethods"), "payment_methods");
        assert!(is_ctx("&Ctx") && is_ctx("& seiva_http :: Ctx") && !is_ctx("RefundPayment"));
        let params = [
            Param {
                name: "ctx",
                ty: "&Ctx",
            },
            Param {
                name: "id",
                ty: "String",
            },
        ];
        assert_eq!(
            input_schema(&params),
            json!({"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"], "additionalProperties": false})
        );
        assert_eq!(
            input_schema(&[]),
            json!({"type": "object", "properties": {}, "additionalProperties": false})
        );
    }
}
