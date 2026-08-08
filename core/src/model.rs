use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum Source {
    HumbleBundle,
    Packt,
    Manning,
    Kindle,
    Manual,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::HumbleBundle => "humble_bundle",
            Source::Packt => "packt",
            Source::Manning => "manning",
            Source::Kindle => "kindle",
            Source::Manual => "manual",
        }
    }
}

impl Serialize for Source {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `f.pad` (rather than a plain `write!`) so width/alignment format
        // specifiers like `{source:<14}` in the `list`/`stats` output work.
        f.pad(self.as_str())
    }
}

impl FromStr for Source {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "humble_bundle" => Ok(Source::HumbleBundle),
            "packt" => Ok(Source::Packt),
            "manning" => Ok(Source::Manning),
            "kindle" => Ok(Source::Kindle),
            "manual" => Ok(Source::Manual),
            other => Err(anyhow!(
                "unknown source '{other}' (expected one of: humble_bundle, packt, manning, kindle, manual)"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Book {
    pub id: Option<i64>,
    pub title: String,
    pub authors: Vec<String>,
    pub isbn: Option<String>,
    pub source: Source,
    pub source_id: Option<String>,
    pub formats: Vec<String>,
    pub acquired_at: Option<chrono::NaiveDate>,
    pub raw_json: Option<String>,
}
