use std::path::PathBuf;

use modeloid::Workspace;

fn main() {
    let mut new_ws = Workspace::new(String::from("New Workspace"), PathBuf::from("."));
    new_ws.save();
    // println!("{new_ws:?}");
    // new_ws.set_description(String::from("A new workspace"));
    // println!("{new_ws:?}");
}   
