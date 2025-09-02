use serde::Serialize;

#[derive(Serialize)]
pub struct ListItem {
    pub name: String,
    pub playlist: String,
}
