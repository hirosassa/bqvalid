pub struct TableInfo {
    pub(crate) table_name: String,
    pub(crate) alias_name: Option<String>,
}

impl TableInfo {
    pub(crate) const fn new(table_name: String, alias_name: Option<String>) -> Self {
        Self {
            table_name,
            alias_name,
        }
    }
}
