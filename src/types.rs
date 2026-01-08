use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct SubPartition {
    pub id: String,
    pub name: String,
    #[serde(rename = "parent_name")]
    pub _parent_name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ParentPartition {
    #[serde(rename = "id")]
    pub _id: i64,
    pub name: String,
    #[serde(rename = "list")]
    pub children: Vec<SubPartition>,
}
