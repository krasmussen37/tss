/// Filters that can be applied to search/list queries.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub speaker: Option<String>,
    pub source: Option<String>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub tag: Option<String>,
}

impl Filters {
    /// Build WHERE clause fragments and params for transcript-level queries.
    /// Returns (clause_parts, param_values) where clause_parts are AND-able conditions.
    pub fn transcript_conditions(&self) -> (Vec<String>, Vec<Box<dyn rusqlite::types::ToSql>>) {
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref source) = self.source {
            conditions.push(format!("t.source = ?{}", params.len() + 1));
            params.push(Box::new(source.clone()));
        }

        if let Some(ref from) = self.from_date {
            conditions.push(format!("t.date >= ?{}", params.len() + 1));
            params.push(Box::new(from.clone()));
        }

        if let Some(ref to) = self.to_date {
            conditions.push(format!("t.date <= ?{}", params.len() + 1));
            params.push(Box::new(to.clone()));
        }

        if let Some(ref speaker) = self.speaker {
            conditions.push(format!(
                "t.id IN (SELECT transcript_id FROM speakers WHERE name LIKE ?{})",
                params.len() + 1
            ));
            params.push(Box::new(format!("%{speaker}%")));
        }

        if let Some(ref tag) = self.tag {
            conditions.push(format!(
                "t.id IN (SELECT transcript_id FROM tags WHERE tag = ?{})",
                params.len() + 1
            ));
            params.push(Box::new(tag.clone()));
        }

        (conditions, params)
    }
}
