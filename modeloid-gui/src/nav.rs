//! Sidebar navigation structure. Add, rename or reorder tools here;
//! the sidebar and the tool pages both read from `NAV`.

use crate::components::IconName;
use crate::routes::Route;

#[derive(Debug, PartialEq)]
pub struct NavItem {
    pub label: &'static str,
    /// URL segment, e.g. `/dataset/download`.
    pub slug: &'static str,
}

#[derive(Debug, PartialEq)]
pub struct NavCategory {
    pub label: &'static str,
    pub slug: &'static str,
    pub icon: IconName,
    pub items: &'static [NavItem],
}

impl NavCategory {
    pub fn route(&self, item: &NavItem) -> Route {
        Route::Tool {
            section: self.slug.to_string(),
            tool: item.slug.to_string(),
        }
    }
}

const fn item(label: &'static str, slug: &'static str) -> NavItem {
    NavItem { label, slug }
}

pub const NAV: &[NavCategory] = &[
    NavCategory {
        label: "Dataset Manager",
        slug: "dataset",
        icon: IconName::Database,
        items: &[
            item("Datasets", "datasets"),
            item("Explore", "explore"),
            item("Health Check", "health-check"),
            item("Classes & Merge", "classes-merge"),
            item("Split & Balance", "split-balance"),
            item("Convert", "convert"),
            item("Transform", "transform"),
        ],
    },
    NavCategory {
        label: "Training",
        slug: "training",
        icon: IconName::Cpu,
        items: &[
            item("Train Model", "train"),
            item("Test Model", "test"),
            item("Visualize Results", "visualize-results"),
            item("Tune Hyperparameters", "tune"),
            item("Test Pretrained Models", "pretrained"),
        ],
    },
];

/// Finds the category and tool for a `/:section/:tool` route.
pub fn find(section: &str, tool: &str) -> Option<(&'static NavCategory, &'static NavItem)> {
    let category = NAV.iter().find(|c| c.slug == section)?;
    let item = category.items.iter().find(|i| i.slug == tool)?;
    Some((category, item))
}
