use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use serde_json::Value;

pub fn json_to_xml(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    key: &str,
    value: &Value,
) -> Result<(), quick_xml::Error> {
    match value {
        Value::Object(map) => {
            let mut start_tag = BytesStart::new(key);
            let mut children = Vec::new();

            for (k, v) in map {
                if v.is_null() {
                    continue;
                }
                if v.is_object() || v.is_array() {
                    children.push((k, v));
                } else {
                    let v_str = match v {
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::String(s) => s.clone(),
                        _ => String::new(),
                    };
                    start_tag.push_attribute((k.as_str(), v_str.as_str()));
                }
            }

            if children.is_empty() {
                writer.write_event(Event::Empty(start_tag))?;
            } else {
                writer.write_event(Event::Start(start_tag))?;
                for (child_key, child_val) in children {
                    json_to_xml(writer, child_key, child_val)?;
                }
                writer.write_event(Event::End(BytesEnd::new(key)))?;
            }
        }
        Value::Array(vec) => {
            for item in vec {
                json_to_xml(writer, key, item)?;
            }
        }
        _ => {
            let v_str = match value {
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                _ => String::new(),
            };
            writer.write_event(Event::Start(BytesStart::new(key)))?;
            writer.write_event(Event::Text(BytesText::new(&v_str)))?;
            writer.write_event(Event::End(BytesEnd::new(key)))?;
        }
    }
    Ok(())
}
