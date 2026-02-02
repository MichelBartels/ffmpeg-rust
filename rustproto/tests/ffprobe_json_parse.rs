use rsproto::FfprobeJson;

#[test]
fn parse_ffprobe_json_reference() {
    let data = include_str!("../../tests/ref/fate/ffprobe_json");
    let parsed: FfprobeJson = serde_json::from_str(data).expect("parse json reference");
    assert_eq!(parsed.format.format_name, "nut");
    assert_eq!(parsed.format.nb_streams, 3);
    assert!(!parsed.streams.is_empty());
}
