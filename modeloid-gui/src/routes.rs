use dioxus::prelude::*;

use crate::components::AppLayout;
use crate::pages::{CreateWorkspace, Home, Label, NotFound, Settings, Tool};

/// Every page of the app. Tool pages share one route; `section` and `tool`
/// are slugs looked up in `nav::NAV`.
#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(AppLayout)]
        #[route("/")]
        Home {},

        #[route("/workspace/create")]
        CreateWorkspace {},

        #[route("/settings")]
        Settings {},

        #[route("/label/:id/:index")]
        Label { id: u32, index: usize },

        #[route("/:section/:tool")]
        Tool { section: String, tool: String },
    #[end_layout]

    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_routes() {
        assert_eq!("/workspace/create".parse::<Route>().unwrap(), Route::CreateWorkspace {});
        assert_eq!("/settings".parse::<Route>().unwrap(), Route::Settings {});
        assert_eq!(
            "/dataset/datasets".parse::<Route>().unwrap(),
            Route::Tool { section: "dataset".into(), tool: "datasets".into() }
        );
        assert_eq!("/label/3/12".parse::<Route>().unwrap(), Route::Label { id: 3, index: 12 });
    }
}
