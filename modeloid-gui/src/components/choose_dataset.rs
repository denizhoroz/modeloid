use dioxus::prelude::*;

use crate::components::DatasetPicker;
use crate::state::use_selected_dataset;

/// Shown by single-dataset tools while nothing is selected: pick here, or
/// click a row in the datasets panel. The choice becomes the app-wide selection.
#[component]
pub fn ChooseDataset() -> Element {
    let mut selected = use_selected_dataset();
    let picked = use_signal(Vec::<u32>::new);

    use_effect(move || {
        if let Some(id) = picked.read().first() {
            selected.set(Some(*id));
        }
    });

    rsx! {
        DatasetPicker { title: "Dataset", selected: picked }
    }
}
