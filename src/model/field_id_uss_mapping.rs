/// Model for `TCG_UCS.FIELD_ID_USS_MAPPING` table rows.
///
/// Mirrors Go's `internal/model/field_id_uss_mapping.go`.

#[derive(Debug, Clone)]
pub struct FieldIdUssMapping {
    pub id: i64,
    pub mcs_id: i64,
    pub field_id: String,
    pub field_name: String,
    pub uss_id: i32,
    pub create_time: Option<chrono::NaiveDateTime>,
    pub update_time: Option<chrono::NaiveDateTime>,
}
