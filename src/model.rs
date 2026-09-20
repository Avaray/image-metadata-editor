use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Default, Debug)]
pub struct Output<'a> {
    pub file: &'a str,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub directories: BTreeMap<String, BTreeMap<String, String>>,
}
