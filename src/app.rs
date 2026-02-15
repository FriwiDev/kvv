use leptos::task::spawn_local;
use leptos::{ev::{SubmitEvent, MouseEvent}, prelude::*};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use js_sys::JSON;
use leptos::web_sys::console;
use crate::efa::stopfinder;

#[wasm_bindgen]
extern "C" {
    // Use `catch` so JS exceptions (e.g. plugin errors) are returned as Err(JsValue)
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

#[derive(Serialize, Deserialize)]
struct GreetArgs<'a> {
    name: &'a str,
}

#[component]
pub fn App() -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (greet_msg, set_greet_msg) = signal(String::new());
    // Signal to hold the geolocation result (printed to UI)
    let (pos_msg, set_pos_msg) = signal(String::new());
    // Signal to hold station search results as display strings
    let (stations, set_stations) = signal(Vec::<String>::new());

    let update_name = move |ev| {
        let v = event_target_value(&ev);
        set_name.set(v);
    };

    let greet = move |ev: SubmitEvent| {
        ev.prevent_default();
        spawn_local(async move {
            let q = name.get_untracked();
            if q.is_empty() {
                set_greet_msg.set("Please enter a station name.".to_string());
                return;
            }
            set_greet_msg.set("Searching stations...".to_string());
            match stopfinder(&q, 10).await {
                Ok(list) => {
                    if list.is_empty() {
                        set_greet_msg.set("No stations found.".to_string());
                        set_stations.set(Vec::new());
                    } else {
                        set_greet_msg.set(format!("Found {} stations", list.len()));
                        let formatted: Vec<String> = list
                            .into_iter()
                            .map(|s| {
                                let place = s.place.unwrap_or_default();
                                if place.is_empty() {
                                    format!("{} — {}", s.name, s.id)
                                } else {
                                    format!("{} ({}) — {}", s.name, place, s.id)
                                }
                            })
                            .collect();
                        set_stations.set(formatted);
                    }
                }
                Err(e) => {
                    set_greet_msg.set(format!("Search failed: {}", e));
                    set_stations.set(Vec::new());
                }
            }
        });
    };

    let get_position = move |_: MouseEvent| {
        spawn_local(async move {
            // inner logic reused from below
            let is_granted = |val: &serde_json::Value| -> bool {
                if let Some(loc) = val.get("location").and_then(|v| v.as_str()) {
                    return !matches!(loc, "prompt" | "prompt-with-rationale")
                }
                false
            };

            set_pos_msg.set("Checking permissions...".to_string());

            // 1) Check permissions
            let check_cmd = "plugin:geolocation|check_permissions";
            let mut granted = false;
            match invoke(check_cmd, JsValue::NULL).await {
                Ok(jsv) => {
                    if let Ok(val) = serde_wasm_bindgen::from_value::<serde_json::Value>(jsv) {
                        // If the plugin explicitly asks for a rationale, show it to the user.
                        if let Some(loc) = val.get("location").and_then(|v| v.as_str()) {
                            if loc == "prompt-with-rationale" {
                                set_pos_msg.set("Location permission requires a rationale: please allow location access when prompted.".to_string());
                            }
                        }
                        granted = is_granted(&val);
                    }
                }
                Err(e) => {
                    console::log_1(&e);
                    if let Ok(sj) = JSON::stringify(&e) {
                        console::log_1(&sj.into());
                    }
                    let s = js_value_to_string(&e);
                    set_pos_msg.set(format!("check_permissions error: {}", s));
                    return;
                }
            }

            // 2) If not granted, request permissions
            if !granted {
                set_pos_msg.set("Requesting permissions...".to_string());
                match invoke("plugin:geolocation|request_permissions", JsValue::NULL).await {
                    Ok(jsv) => {
                        if let Ok(val) = serde_wasm_bindgen::from_value::<serde_json::Value>(jsv) {
                            granted = is_granted(&val);
                        } else {
                            set_pos_msg.set("Permission request response could not be parsed; aborting.".to_string());
                            return;
                        }
                    }
                    Err(e) => {
                        console::log_1(&e);
                        if let Ok(sj) = JSON::stringify(&e) {
                            console::log_1(&sj.into());
                        }
                        let s = js_value_to_string(&e);
                        set_pos_msg.set(format!("request_permissions error: {}", s));
                        return;
                    }
                }
            }

            if !granted {
                set_pos_msg.set("Permissions not granted.".to_string());
                return;
            }

            // 3) Now request the current position
            set_pos_msg.set("Getting current position...".to_string());
            match invoke("plugin:geolocation|get_current_position", JsValue::NULL).await {
                Ok(jsv) => {
                    match serde_wasm_bindgen::from_value::<serde_json::Value>(jsv) {
                        Ok(val) => {
                            if let Some(coords) = val.get("coords") {
                                if let (Some(lon), Some(lat)) = (coords.get("longitude").and_then(|v| v.as_f64()), coords.get("latitude").and_then(|v| v.as_f64())) {
                                    set_pos_msg.set(format!("Current position: longitude {}, latitude {}", lon, lat));
                                    return;
                                }
                            }
                            set_pos_msg.set(format!("Invalid value received: {val}"));
                        }
                        Err(e) => {
                            set_pos_msg.set(format!("Error parsing position: {}", e));
                        }
                    }
                }
                Err(e) => {
                    console::log_1(&e);
                    if let Ok(sj) = JSON::stringify(&e) {
                        console::log_1(&sj.into());
                    }
                    let s = js_value_to_string(&e);
                    set_pos_msg.set(format!("get_current_position error: {}", s));
                }
            }
        });
    };

    // Load position once on app startup (component mount)
    {
        let set_pos_msg_start = set_pos_msg.clone();
        spawn_local(async move {
            // replicate the same flow as above but using the cloned setter
            let is_granted = |val: &serde_json::Value| -> bool {
                if let Some(loc) = val.get("location").and_then(|v| v.as_str()) {
                    return !matches!(loc, "prompt" | "prompt-with-rationale")
                }
                false
            };

            set_pos_msg_start.set("Checking permissions...".to_string());

            // 1) Check permissions
            let check_cmd = "plugin:geolocation|check_permissions";
            let mut granted = false;
            match invoke(check_cmd, JsValue::NULL).await {
                Ok(jsv) => {
                    if let Ok(val) = serde_wasm_bindgen::from_value::<serde_json::Value>(jsv) {
                        if let Some(loc) = val.get("location").and_then(|v| v.as_str()) {
                            if loc == "prompt-with-rationale" {
                                set_pos_msg_start.set("Location permission requires a rationale: please allow location access when prompted.".to_string());
                            }
                        }
                        granted = is_granted(&val);
                    }
                }
                Err(e) => {
                    console::log_1(&e);
                    if let Ok(sj) = JSON::stringify(&e) {
                        console::log_1(&sj.into());
                    }
                    let s = js_value_to_string(&e);
                    set_pos_msg_start.set(format!("check_permissions error: {}", s));
                    return;
                }
            }

            // 2) If not granted, request permissions
            if !granted {
                set_pos_msg_start.set("Requesting permissions...".to_string());
                match invoke("plugin:geolocation|request_permissions", JsValue::NULL).await {
                    Ok(jsv) => {
                        if let Ok(val) = serde_wasm_bindgen::from_value::<serde_json::Value>(jsv) {
                            granted = is_granted(&val);
                        } else {
                            set_pos_msg_start.set("Permission request response could not be parsed; aborting.".to_string());
                            return;
                        }
                    }
                    Err(e) => {
                        console::log_1(&e);
                        if let Ok(sj) = JSON::stringify(&e) {
                            console::log_1(&sj.into());
                        }
                        let s = js_value_to_string(&e);
                        set_pos_msg_start.set(format!("request_permissions error: {}", s));
                        return;
                    }
                }
            }

            if !granted {
                set_pos_msg_start.set("Permissions not granted.".to_string());
                return;
            }

            // 3) Now request the current position
            set_pos_msg_start.set("Getting current position...".to_string());
            match invoke("plugin:geolocation|get_current_position", JsValue::NULL).await {
                Ok(jsv) => {
                    match serde_wasm_bindgen::from_value::<serde_json::Value>(jsv) {
                        Ok(val) => {
                            if let Some(coords) = val.get("coords") {
                                if let (Some(lon), Some(lat)) = (coords.get("longitude").and_then(|v| v.as_f64()), coords.get("latitude").and_then(|v| v.as_f64())) {
                                    set_pos_msg_start.set(format!("Current position: longitude {}, latitude {}", lon, lat));
                                    return;
                                }
                            }
                            set_pos_msg_start.set(format!("Invalid value received: {val}"));
                        }
                        Err(e) => {
                            set_pos_msg_start.set(format!("Error parsing position: {}", e));
                        }
                    }
                }
                Err(e) => {
                    console::log_1(&e);
                    if let Ok(sj) = JSON::stringify(&e) {
                        console::log_1(&sj.into());
                    }
                    let s = js_value_to_string(&e);
                    set_pos_msg_start.set(format!("get_current_position error: {}", s));
                }
            }
        });
    }

    view! {
        <main class="container">
            <h1>"Welcome to Tauri + Leptos"</h1>

            <div class="row">
                <a href="https://tauri.app" target="_blank">
                    <img src="public/tauri.svg" class="logo tauri" alt="Tauri logo"/>
                </a>
                <a href="https://docs.rs/leptos/" target="_blank">
                    <img src="public/leptos.svg" class="logo leptos" alt="Leptos logo"/>
                </a>
            </div>
            <p>"Click on the Tauri and Leptos logos to learn more."</p>

            <form class="row" on:submit=greet>
                <input
                    id="greet-input"
                    placeholder="Station name..."
                    on:input=update_name
                />
                <button type="submit">"Search Stations"</button>
                <button type="button" on:click=get_position>"Get Position"</button>
            </form>
            <p>{ move || greet_msg.get() }</p>
            <ul>
                { move || stations.get().iter().map(|s| {
                    let s = s.clone();
                    view! { <li>{ s }</li> }
                }).collect::<Vec<_>>() }
            </ul>
            <pre>{ move || pos_msg.get() }</pre>
        </main>
    }
}

// Convert a JsValue (string/number/object) into a readable String
fn js_value_to_string(v: &JsValue) -> String {
    if v.is_string() {
        return v.as_string().unwrap_or_default();
    }
    if v.as_f64().is_some() {
        if let Some(n) = v.as_f64() {
            return n.to_string();
        }
    }
    match JSON::stringify(v) {
        Ok(s) => s.as_string().unwrap_or_else(|| format!("{:?}", v)),
        Err(_) => v.as_string().unwrap_or_else(|| format!("{:?}", v)),
    }
}
