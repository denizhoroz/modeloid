use dioxus::prelude::*;

mod api;
mod components;
mod nav;
mod pages;
mod routes;
mod state;
mod util;

use components::Fonts;
use routes::Route;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Title { "Modeloid" }
        document::Link { rel: "icon", href: FAVICON }
        Fonts {}
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        Router::<Route> {}
    }
}
