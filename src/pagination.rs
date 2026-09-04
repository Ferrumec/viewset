use actixutils::Filters;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum SortDirection {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

pub struct PaginationParams {
    pub limit: u32,
    pub offset: u32,
    pub page: u32,
}

impl PaginationParams {
    pub const DEFAULT_PAGE_SIZE: &'static str = "25";
    pub const MAX_PAGE_SIZE: u32 = 200;

    pub fn from_query(q: &Filters) -> Self {
        let page: u32 = q
            .get("page")
            .unwrap_or(&"1".to_string())
            .parse()
            .unwrap_or(1);
        let limit: u32 = q
            .get("page_size")
            .unwrap_or(&Self::DEFAULT_PAGE_SIZE.to_string())
            .parse()
            .unwrap_or(25);
        let limit = limit.clamp(1, Self::MAX_PAGE_SIZE);
        Self {
            limit,
            offset: (page - 1) * limit,
            page,
        }
    }

    pub fn parse_sort(raw: &str) -> Vec<(String, SortDirection)> {
        raw.split(',')
            .filter(|s| !s.is_empty())
            .map(|s| {
                if let Some(field) = s.strip_prefix('-') {
                    (field.to_string(), SortDirection::Desc)
                } else {
                    (s.to_string(), SortDirection::Asc)
                }
            })
            .collect()
    }
}

#[derive(Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub page_size: u32,
    pub total: i64,
    pub total_pages: u32,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, pagination: &PaginationParams, total: i64) -> Self {
        let total_pages = ((total as f64) / (pagination.limit as f64)).ceil().max(1.0) as u32;
        Self {
            items,
            page: pagination.page,
            page_size: pagination.limit,
            total,
            total_pages,
        }
    }
}
