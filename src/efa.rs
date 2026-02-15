use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::Value;
use html_escape::decode_html_entities;
use serde_urlencoded;

/// Cross-platform fetch helper: uses gloo-net on wasm32 and reqwest otherwise
async fn fetch_text(url: &str, params: &Vec<(&str, String)>) -> Result<String, String> {
    // serialize params into query string
    let qpairs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let query = serde_urlencoded::to_string(&qpairs).map_err(|e| e.to_string())?;
    let full = if query.is_empty() { url.to_string() } else { format!("{}?{}", url, query) };

    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        let resp = Request::get(&full).send().await.map_err(|e| e.to_string())?;
        let txt = resp.text().await.map_err(|e| e.to_string())?;
        Ok(txt)
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let client = reqwest::Client::new();
        let resp = client.get(&full).send().await.map_err(|e| e.to_string())?;
        let txt = resp.text().await.map_err(|e| e.to_string())?;
        Ok(txt)
    }
}

const API_BASE: &str = "https://projekte.kvv-efa.de/sl3/";

#[derive(Clone, Debug, PartialEq)]
pub struct StopSuggestion {
    pub id: String,
    pub name: String,
    pub place: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Departure {
    pub line: String,
    pub direction: Option<String>,
    pub time: String,
    pub planned_time: String,
    pub realtime_time: Option<String>,
}

fn common_params() -> Vec<(&'static str, String)> {
    vec![
        ("language", "de".to_string()),
        ("stateless", "1".to_string()),
        ("coordOutputFormat", "WGS84[DD.ddddd]".to_string()),
        ("coordOutputFormatTail", "7".to_string()),
    ]
}

pub async fn stopfinder(query: &str, max: usize) -> Result<Vec<StopSuggestion>, String> {
    let mut params = common_params();
    params.push(("outputFormat", "JSON".to_string()));
    params.push(("locationServerActive", "1".to_string()));
    params.push(("regionID_sf", "1".to_string()));
    params.push(("type_sf", "any".to_string()));
    params.push(("name_sf", query.to_string()));
    params.push(("anyObjFilter_sf", "2".to_string())); // stops only
    params.push(("reducedAnyPostcodeObjFilter_sf", "64".to_string()));
    params.push(("reducedAnyTooManyObjFilter_sf", "2".to_string()));
    params.push(("useHouseNumberList", "true".to_string()));
    params.push(("anyMaxSizeHitList", max.to_string()));

    let url = format!("{API_BASE}XML_STOPFINDER_REQUEST");
    let body = fetch_text(&url, &params).await?;
    parse_stopfinder_json(&body)
}

fn parse_stop_point(point: &Value) -> Option<StopSuggestion> {
    let typ = point.get("type")?.as_str()?;
    let typ = if typ == "any" {
        point.get("anyType")?.as_str()?
    } else {
        typ
    };
    if typ != "stop" {
        return None;
    }
    let name = decode_text(point.get("name")?.as_str()?);
    let reference = point.get("ref")?;
    let id = reference.get("id")?.as_str()?.to_string();
    let place = reference
        .get("place")
        .and_then(|p| p.as_str())
        .map(decode_text)
        .filter(|p| !p.is_empty());
    Some(StopSuggestion { id, name, place })
}

fn parse_stopfinder_json(body: &str) -> Result<Vec<StopSuggestion>, String> {
    let json: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let points = json
        .get("stopFinder")
        .and_then(|sf| sf.get("points"))
        .or_else(|| json.get("stopFinder"));

    let mut stops = Vec::new();
    match points {
        Some(Value::Object(map)) => {
            if let Some(point) = map.get("point") {
                if let Some(stop) = parse_stop_point(point) {
                    stops.push(stop);
                }
            }
        }
        Some(Value::Array(arr)) => {
            for item in arr {
                if let Some(stop) = parse_stop_point(item) {
                    stops.push(stop);
                }
            }
        }
        _ => {}
    }

    Ok(stops)
}

pub async fn departures(station_id: &str, max: usize) -> Result<Vec<Departure>, String> {
    let mut params = common_params();
    params.push(("outputFormat", "XML".to_string()));
    params.push(("type_dm", "stop".to_string()));
    params.push(("name_dm", station_id.to_string()));
    params.push(("useRealtime", "1".to_string()));
    params.push(("mode", "direct".to_string()));
    params.push(("ptOptionsActive", "1".to_string()));
    params.push(("deleteAssignedStops_dm", "1".to_string()));
    params.push(("useProxFootSearch", "0".to_string()));
    params.push(("mergeDep", "1".to_string()));
    params.push(("limit", max.to_string()));

    let url = format!("{API_BASE}XSLT_DM_REQUEST");
    let body = fetch_text(&url, &params).await?;
    parse_departures_xml(&body)
}

pub async fn departures_live(station_id: &str, max: usize) -> Result<Vec<Departure>, String> {
    departures(station_id, max).await
}

fn parse_departures_xml(xml: &str) -> Result<Vec<Departure>, String> {
    let mut reader = Reader::from_str(xml);

    let mut buf = Vec::new();
    let mut in_departure = false;
    let mut in_datetime = false;
    let mut in_rt_datetime = false;

    let mut current_line: Option<String> = None;
    let mut current_direction: Option<String> = None;
    let mut current_time: Option<String> = None;
    let mut planned_time: Option<String> = None;
    let mut realtime_time: Option<String> = None;
    let mut departures = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"itdDeparture" => {
                    in_departure = true;
                    current_line = None;
                    current_direction = None;
                    current_time = None;
                    planned_time = None;
                    realtime_time = None;
                }
                b"itdDateTime" if in_departure => {
                    if current_time.is_none() {
                        in_datetime = true;
                    }
                }
                b"itdRTDateTime" if in_departure => {
                    if realtime_time.is_none() {
                        in_rt_datetime = true;
                    }
                }
                b"itdTime" if in_departure && in_datetime => {
                    if let Some(t) = parse_time_from_attrs(&e) {
                        planned_time = Some(t.clone());
                        if realtime_time.is_none() {
                            current_time = Some(t);
                        }
                    }
                }
                b"itdTime" if in_departure && in_rt_datetime => {
                    if let Some(t) = parse_time_from_attrs(&e) {
                        realtime_time = Some(t.clone());
                        current_time = Some(t);
                    }
                }
                b"itdServingLine" if in_departure => {
                    parse_serving_line_attrs(&e, &mut current_line, &mut current_direction);
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"itdTime" if in_departure && in_datetime => {
                    if let Some(t) = parse_time_from_attrs(&e) {
                        planned_time = Some(t.clone());
                        if realtime_time.is_none() {
                            current_time = Some(t);
                        }
                    }
                }
                b"itdTime" if in_departure && in_rt_datetime => {
                    if let Some(t) = parse_time_from_attrs(&e) {
                        realtime_time = Some(t.clone());
                        current_time = Some(t);
                    }
                }
                b"itdServingLine" if in_departure => {
                    parse_serving_line_attrs(&e, &mut current_line, &mut current_direction);
                }
                _ => {}
            },
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"itdDateTime" => {
                    in_datetime = false;
                }
                b"itdRTDateTime" => {
                    in_rt_datetime = false;
                }
                b"itdDeparture" => {
                    if let (Some(line), Some(time), Some(planned)) = (
                        current_line.take(),
                        current_time.take(),
                        planned_time.take(),
                    ) {
                        departures.push(Departure {
                            line,
                            direction: current_direction.take(),
                            time,
                            planned_time: planned,
                            realtime_time: realtime_time.take(),
                        });
                    }
                    in_departure = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
        buf.clear();
    }

    Ok(departures)
}

fn parse_time_from_attrs(e: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    let mut hour = None;
    let mut minute = None;
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            b"hour" => hour = Some(String::from_utf8_lossy(&attr.value).to_string()),
            b"minute" => minute = Some(String::from_utf8_lossy(&attr.value).to_string()),
            _ => {}
        }
    }
    if let (Some(h), Some(m)) = (hour, minute) {
        if let (Ok(hh), Ok(mm)) = (h.parse::<u8>(), m.parse::<u8>()) {
            return Some(format!("{hh:02}:{mm:02}"));
        }
    }
    None
}

fn parse_serving_line_attrs(
    e: &quick_xml::events::BytesStart<'_>,
    current_line: &mut Option<String>,
    current_direction: &mut Option<String>,
) {
    let mut symbol = None;
    let mut number = None;
    let mut direction = None;
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            b"symbol" => symbol = Some(String::from_utf8_lossy(&attr.value).to_string()),
            b"number" => number = Some(String::from_utf8_lossy(&attr.value).to_string()),
            b"direction" => direction = Some(decode_text(&String::from_utf8_lossy(&attr.value))),
            _ => {}
        }
    }
    *current_line = symbol.or(number);
    *current_direction = direction;
}

fn decode_text(input: &str) -> String {
    decode_html_entities(input).to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse_departures_xml, parse_stopfinder_json, departures, stopfinder};
    use tokio::time::{timeout, Duration};

    #[test]
    fn parse_departures_xml_extracts_line_time_direction() {
        let xml = r#"
            <itdRequest>
              <itdDepartureMonitorRequest>
                <itdDepartureList>
                  <itdDeparture stopID="1001">
                    <itdDateTime>
                      <itdDate year="2024" month="01" day="01" weekday="1" />
                      <itdTime hour="08" minute="05" />
                    </itdDateTime>
                    <itdRTDateTime>
                      <itdDate year="2024" month="01" day="01" weekday="1" />
                      <itdTime hour="08" minute="07" />
                    </itdRTDateTime>
                    <itdServingLine symbol="S1" direction="Hbf" motType="1" />
                  </itdDeparture>
                  <itdDeparture stopID="1002">
                    <itdDateTime>
                      <itdDate year="2024" month="01" day="01" weekday="1" />
                      <itdTime hour="09" minute="30" />
                    </itdDateTime>
                    <itdServingLine number="2" direction="Durlach" motType="3" />
                  </itdDeparture>
                </itdDepartureList>
              </itdDepartureMonitorRequest>
            </itdRequest>
        "#;

        let departures = parse_departures_xml(xml).expect("parse succeeds");
        assert_eq!(departures.len(), 2);

        assert_eq!(departures[0].time, "08:07");
        assert_eq!(departures[0].planned_time, "08:05");
        assert_eq!(departures[0].realtime_time.as_deref(), Some("08:07"));
        assert_eq!(departures[0].line, "S1");
        assert_eq!(departures[0].direction.as_deref(), Some("Hbf"));

        assert_eq!(departures[1].time, "09:30");
        assert_eq!(departures[1].planned_time, "09:30");
        assert_eq!(departures[1].realtime_time, None);
        assert_eq!(departures[1].line, "2");
        assert_eq!(departures[1].direction.as_deref(), Some("Durlach"));
    }

    #[test]
    fn parse_stopfinder_json_extracts_stop_suggestions() {
        let json = r#"
        {
          "stopFinder": {
            "points": [
              {
                "type": "stop",
                "name": "Karlsruhe Hbf",
                "ref": {
                  "id": "7000101",
                  "place": "Karlsruhe"
                }
              },
              {
                "type": "poi",
                "name": "Zoo",
                "ref": {
                  "id": "poi-1",
                  "place": "Karlsruhe"
                }
              }
            ]
          }
        }
        "#;

        let stops = parse_stopfinder_json(json).expect("parse succeeds");
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].id, "7000101");
        assert_eq!(stops[0].name, "Karlsruhe Hbf");
        assert_eq!(stops[0].place.as_deref(), Some("Karlsruhe"));
    }

    #[tokio::test]
    async fn live_stopfinder_returns_results() {
        let result = timeout(Duration::from_secs(15), stopfinder("Karlsruhe, ZKM", 5))
            .await
            .expect("stopfinder timed out")
            .expect("stopfinder request failed");
        assert!(!result.is_empty(), "expected stopfinder results");
    }

    #[tokio::test]
    async fn live_departures_returns_results() {
        let stops = timeout(Duration::from_secs(15), stopfinder("Karlsruhe, ZKM", 1))
            .await
            .expect("stopfinder timed out")
            .expect("stopfinder request failed");
        let stop = stops.first().expect("expected at least one stop");
        let departures = timeout(Duration::from_secs(15), departures(&stop.id, 5))
            .await
            .expect("departures timed out")
            .expect("departures request failed");
        assert!(!departures.is_empty(), "expected departures");
    }
}