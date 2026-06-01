//! Output rendering : table / json / yaml.
//!
//! Each command produces a `Renderable` (typed struct) and asks the
//! shared `Renderer` to format it. Tests assert on JSON shape (most
//! stable) and on table headers.

use std::fmt::Write as _;

use comfy_table::{
    presets::UTF8_BORDERS_ONLY, Cell, ContentArrangement, Table,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AdminError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Table,
    Json,
    Yaml,
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Table
    }
}

#[derive(Clone)]
pub struct Renderer {
    format: OutputFormat,
}

impl Renderer {
    #[must_use]
    pub fn new(format: OutputFormat) -> Self {
        Self { format }
    }

    /// Render any serializable value.
    pub fn render<T: Serialize>(&self, value: &T) -> Result<String, AdminError> {
        match self.format {
            OutputFormat::Json => Ok(serde_json::to_string_pretty(value)?),
            OutputFormat::Yaml => Ok(serde_yaml::to_string(value)?),
            OutputFormat::Table => {
                // Convert to JSON Value first for uniform table builder.
                let v = serde_json::to_value(value)?;
                Ok(render_table(&v))
            }
        }
    }

    /// Same as `render`, but also prints to stdout.
    pub fn print<T: Serialize>(&self, value: &T) -> Result<(), AdminError> {
        let s = self.render(value)?;
        println!("{s}");
        Ok(())
    }

    pub fn format(&self) -> OutputFormat {
        self.format
    }
}

fn render_table(v: &Value) -> String {
    match v {
        Value::Array(items) if !items.is_empty() => render_array_of_objects(items),
        Value::Object(_) => render_single_object(v),
        Value::Null => "<null>".into(),
        other => other.to_string(),
    }
}

fn render_array_of_objects(items: &[Value]) -> String {
    // Collect column union (stable order: by first appearance).
    let mut cols: Vec<String> = Vec::new();
    for item in items {
        if let Value::Object(map) = item {
            for k in map.keys() {
                if !cols.iter().any(|c| c == k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    if cols.is_empty() {
        // Fallback : array of scalars.
        return items.iter().map(stringify).collect::<Vec<_>>().join("\n");
    }

    let mut t = Table::new();
    t.load_preset(UTF8_BORDERS_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(cols.iter().map(|c| Cell::new(c)));

    for item in items {
        if let Value::Object(map) = item {
            t.add_row(cols.iter().map(|c| {
                Cell::new(map.get(c).map_or("-".into(), stringify))
            }));
        }
    }
    let mut out = t.to_string();
    let _ = writeln!(out, "{} rows", items.len());
    out
}

fn render_single_object(v: &Value) -> String {
    let Value::Object(map) = v else { return v.to_string() };
    let mut t = Table::new();
    t.load_preset(UTF8_BORDERS_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["key", "value"]);
    for (k, val) in map {
        t.add_row(vec![Cell::new(k), Cell::new(stringify(val))]);
    }
    t.to_string()
}

fn stringify(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(stringify).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Row {
        name: String,
        rps: u32,
    }

    #[test]
    fn render_array_table_contains_headers() {
        let r = Renderer::new(OutputFormat::Table);
        let rows = vec![
            Row { name: "acme".into(), rps: 100 },
            Row { name: "globex".into(), rps: 200 },
        ];
        let out = r.render(&rows).unwrap();
        assert!(out.contains("name"));
        assert!(out.contains("rps"));
        assert!(out.contains("acme"));
        assert!(out.contains("globex"));
        assert!(out.contains("2 rows"));
    }

    #[test]
    fn render_single_object_table_has_key_value() {
        let r = Renderer::new(OutputFormat::Table);
        let rows = Row { name: "acme".into(), rps: 100 };
        let out = r.render(&rows).unwrap();
        assert!(out.contains("name"));
        assert!(out.contains("acme"));
    }

    #[test]
    fn render_json_pretty() {
        let r = Renderer::new(OutputFormat::Json);
        let rows = Row { name: "acme".into(), rps: 100 };
        let out = r.render(&rows).unwrap();
        assert!(out.contains("\"name\""));
        assert!(out.contains("\"acme\""));
        // pretty-printed → multiline
        assert!(out.contains('\n'));
    }

    #[test]
    fn render_yaml() {
        let r = Renderer::new(OutputFormat::Yaml);
        let rows = Row { name: "acme".into(), rps: 100 };
        let out = r.render(&rows).unwrap();
        assert!(out.contains("name: acme"));
        assert!(out.contains("rps: 100"));
    }

    #[test]
    fn empty_array_does_not_panic() {
        let r = Renderer::new(OutputFormat::Table);
        let rows: Vec<Row> = vec![];
        let _ = r.render(&rows).unwrap();
    }
}
