use anyhow::{Context, Result};
use itertools::Itertools;
use prost::Message;
use prost_reflect::{
    DescriptorPool, DeserializeOptions, DynamicMessage, MessageDescriptor, ReflectMessage,
};
use serde_json::de::Deserializer;
use std::io::Cursor;
use std::path::Path;
use std::{fs, path::PathBuf};
use tempfile::{self, TempDir};

pub trait ProtobufDescriptorLoader {
    fn build_protobuf_descriptor(proto_string: &String) -> Result<DescriptorPool> {
        let (tempdir, tempfile) =
            Self::_store_temp_proto_file(&"temp.proto".to_string(), proto_string)
                .context("on storing temp proto file")?;
        let descriptor_file = tempdir.path().join("descriptor.bin");
        tonic_prost_build::configure()
            // only output message descriptor
            .build_server(false)
            .build_client(false)
            .build_transport(false)
            .out_dir(&tempdir)
            .protoc_arg("--experimental_allow_proto3_optional")
            .file_descriptor_set_path(&descriptor_file) // for reflection
            .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
            .compile_protos(&[&tempfile], &[&tempdir.path().to_path_buf()])
            .context(format!("Failed to compile protos {:?}", &tempfile))?;

        let descriptor = Self::_load_protobuf_descriptor(&descriptor_file)?;
        Ok(descriptor)
    }

    fn _load_protobuf_descriptor(descriptor_file: &Path) -> Result<DescriptorPool> {
        let descriptor_bytes = fs::read(descriptor_file).context(format!(
            "on reading descriptor file: {:?}",
            descriptor_file.to_str()
        ))?;
        let descriptor_pool = DescriptorPool::decode(descriptor_bytes.as_ref())
            .context("on decoding descriptor bytes")?;
        Ok(descriptor_pool)
    }

    fn _store_temp_proto_file(
        proto_name: &String,
        proto_string: &String,
    ) -> Result<(TempDir, PathBuf)> {
        let temp_dir =
            tempfile::tempdir().context(format!("on creating tempfile for proto: {proto_name}"))?;
        let tempfile = temp_dir.path().join(proto_name);
        // For now we need to write files to the disk.
        fs::write(&tempfile, proto_string).context(format!(
            "on saving tempfile for proto: {:?}",
            &tempfile.to_str()
        ))?;
        Ok((temp_dir, tempfile))
    }
}

#[derive(Debug, Clone)]
pub struct ProtobufDescriptor {
    pool: DescriptorPool,
}

impl ProtobufDescriptorLoader for ProtobufDescriptor {}
impl ProtobufDescriptor {
    pub fn new(proto_string: &String) -> Result<Self> {
        let pool = ProtobufDescriptor::build_protobuf_descriptor(proto_string)?;
        Ok(ProtobufDescriptor { pool })
    }
    pub fn get_message_names(&self) -> Vec<String> {
        self.pool
            .all_messages()
            .map(|message| message.full_name().to_string())
            .collect()
    }
    pub fn get_messages(&self) -> Vec<MessageDescriptor> {
        self.pool.all_messages().collect()
    }
    pub fn get_message_by_name(&self, message_name: &str) -> Option<MessageDescriptor> {
        self.pool.get_message_by_name(message_name)
    }
    pub fn get_message_from_json(
        descriptor: MessageDescriptor,
        json: &str,
    ) -> Result<DynamicMessage> {
        let mut deserializer = Deserializer::from_str(json);
        let dynamic_message = DynamicMessage::deserialize(descriptor, &mut deserializer)?;
        deserializer.end()?;
        Ok(dynamic_message)
    }
    pub fn get_message_by_name_from_json(
        &self,
        message_name: &str,
        json: &str,
    ) -> Result<DynamicMessage> {
        let message_descriptor = self
            .get_message_by_name(message_name)
            .ok_or(anyhow::anyhow!("message not found by name: {message_name}"))?;
        Self::get_message_from_json(message_descriptor, json)
    }
    pub fn get_message_from_bytes(
        descriptor: MessageDescriptor,
        bytes: &[u8],
    ) -> Result<DynamicMessage> {
        let cursor = std::io::Cursor::new(bytes);
        let dynamic_message = DynamicMessage::decode(descriptor, cursor)?;
        Ok(dynamic_message)
    }
    pub fn get_message_by_name_from_bytes(
        &self,
        message_name: &str,
        bytes: &[u8],
    ) -> Result<DynamicMessage> {
        let message_descriptor = self
            .get_message_by_name(message_name)
            .ok_or(anyhow::anyhow!("message not found by name: {message_name}"))?;
        Self::get_message_from_bytes(message_descriptor, bytes)
    }
    pub fn decode_from_json<T: ReflectMessage + Default>(json: impl AsRef<str>) -> Result<T> {
        let descriptor = T::default().descriptor();
        let mut deserializer = serde_json::Deserializer::from_str(json.as_ref());
        let decoded = DynamicMessage::deserialize(descriptor, &mut deserializer)?;
        deserializer.end()?;
        decoded.transcode_to::<T>().context(format!(
            "decode_from_json: on transcoding dynamic message to {}",
            std::any::type_name::<T>()
        ))
    }
    pub fn serialize_message<T: Message>(arg: &T) -> Vec<u8> {
        let mut buf = Vec::with_capacity(arg.encoded_len());
        arg.encode(&mut buf).unwrap();
        buf
    }
    pub fn deserialize_message<T: Message + Default>(buf: &[u8]) -> Result<T> {
        T::decode(&mut Cursor::new(buf)).map_err(|e| e.into())
    }
    pub fn message_to_json(message: &DynamicMessage) -> Result<String> {
        let json = serde_json::to_string(&message)?;
        Ok(json)
    }
    pub fn message_to_json_value(message: &DynamicMessage) -> Result<serde_json::Value> {
        let json = serde_json::to_value(message)?;
        Ok(json)
    }
    pub fn print_dynamic_message(message: &DynamicMessage, byte_to_string: bool) {
        let message_str = Self::dynamic_message_to_string(message, byte_to_string);
        println!("{message_str}");
    }
    pub fn dynamic_message_to_string(message: &DynamicMessage, byte_to_string: bool) -> String {
        message
            .fields()
            .map(|(field, value)| {
                format!(
                    "{}: {}\n",
                    field.name(),
                    Self::value_to_string(value, byte_to_string)
                )
            })
            .join("")
    }
    fn value_to_string(v: &prost_reflect::Value, byte_to_string: bool) -> String {
        match v {
            prost_reflect::Value::Bool(v) => format!("{v}"),
            prost_reflect::Value::I32(v) => format!("{v}"),
            prost_reflect::Value::I64(v) => format!("{v}"),
            prost_reflect::Value::U32(v) => format!("{v}"),
            prost_reflect::Value::U64(v) => format!("{v}"),
            prost_reflect::Value::F32(v) => format!("{v}"),
            prost_reflect::Value::F64(v) => format!("{v}"),
            prost_reflect::Value::String(v) => v.to_string(),
            prost_reflect::Value::Bytes(v) => {
                if byte_to_string {
                    format!("{}", String::from_utf8_lossy(v))
                } else {
                    format!("{v:x?}")
                }
            }
            prost_reflect::Value::EnumNumber(v) => format!("{v:?}[enum]"),
            prost_reflect::Value::Message(v) => Self::dynamic_message_to_string(v, byte_to_string),
            prost_reflect::Value::List(v) => {
                let list_str = v
                    .iter()
                    .map(|v| Self::value_to_string(v, byte_to_string))
                    .join(", ");
                format!("[{list_str}]")
            }
            prost_reflect::Value::Map(hash_map) => {
                let map_str = hash_map
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}: {}",
                            Self::map_key_to_string(k),
                            Self::value_to_string(v, byte_to_string)
                        )
                    })
                    .join(", ");
                format!("{{{map_str}}}")
            }
        }
    }
    fn map_key_to_string(k: &prost_reflect::MapKey) -> String {
        match k {
            prost_reflect::MapKey::Bool(v) => format!("{v}"),
            prost_reflect::MapKey::I32(v) => format!("{v}"),
            prost_reflect::MapKey::I64(v) => format!("{v}"),
            prost_reflect::MapKey::U32(v) => format!("{v}"),
            prost_reflect::MapKey::U64(v) => format!("{v}"),
            prost_reflect::MapKey::String(v) => v.to_string(),
        }
    }
    pub fn json_value_to_message(
        descriptor: MessageDescriptor,
        json_value: &serde_json::Value,
        ignore_unknown_fields: bool,
    ) -> Result<Vec<u8>> {
        let dynamic_message = if ignore_unknown_fields {
            let options = DeserializeOptions::new().deny_unknown_fields(false);
            DynamicMessage::deserialize_with_options(descriptor, json_value, &options)
        } else {
            DynamicMessage::deserialize(descriptor, json_value)
        }?;
        Ok(dynamic_message.encode_to_vec())
    }
    pub fn json_to_message(descriptor: MessageDescriptor, json_str: &str) -> Result<Vec<u8>> {
        let mut deserializer = Deserializer::from_str(json_str);
        let dynamic_message = DynamicMessage::deserialize(descriptor, &mut deserializer)?;
        deserializer.end()?;
        Ok(dynamic_message.encode_to_vec())
    }

    /// Convert Protobuf MessageDescriptor to JSON Schema
    ///
    /// This utility enables automatic JSON Schema generation from Protobuf definitions.
    /// Plugin developers only need to implement method_proto_map(), and JSON Schema
    /// will be generated automatically via the default implementation of method_json_schema_map().
    ///
    /// # Arguments
    /// * `descriptor` - Protobuf MessageDescriptor to convert
    ///
    /// # Returns
    /// JSON Schema as serde_json::Value
    ///
    /// # Example
    /// ```ignore
    /// let proto_string = r#"
    ///     message CommandArgs {
    ///         string command = 1;
    ///         repeated string args = 2;
    ///         optional int32 timeout_ms = 3;
    ///     }
    /// "#;
    /// let descriptor = ProtobufDescriptor::new(&proto_string)?;
    /// let msg_descriptor = descriptor.get_message_by_name("CommandArgs")?;
    /// let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);
    /// ```
    pub fn message_descriptor_to_json_schema(descriptor: &MessageDescriptor) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required_fields = Vec::new();

        for field in descriptor.fields() {
            let field_schema = Self::field_to_json_schema(&field);
            properties.insert(field.json_name().to_string(), field_schema);

            // Proto3: all fields are optional by default, except for explicitly required
            // For JSON Schema, we treat non-optional, non-repeated, non-map fields as required
            if !field.is_list()
                && !field.is_map()
                && field.cardinality() == prost_reflect::Cardinality::Required
            {
                required_fields.push(field.json_name().to_string());
            }
        }

        let mut schema = serde_json::json!({
            "type": "object",
            "properties": properties,
        });

        if !required_fields.is_empty() {
            schema["required"] = serde_json::Value::Array(
                required_fields
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            );
        }

        schema
    }

    /// Convert Protobuf FieldDescriptor to JSON Schema
    fn field_to_json_schema(field: &prost_reflect::FieldDescriptor) -> serde_json::Value {
        if field.is_list() {
            // Repeated field -> array
            return serde_json::json!({
                "type": "array",
                "items": Self::kind_to_json_schema(&field.kind())
            });
        }

        if field.is_map() {
            // Map field -> object with additionalProperties
            // For map<K, V>, we need to get the value type from the map entry message
            if let prost_reflect::Kind::Message(map_entry) = field.kind() {
                // Map entry has two fields: key (field 1) and value (field 2)
                if let Some(value_field) = map_entry.fields().find(|f| f.number() == 2) {
                    return serde_json::json!({
                        "type": "object",
                        "additionalProperties": Self::kind_to_json_schema(&value_field.kind())
                    });
                }
            }
            // Fallback for malformed map
            return serde_json::json!({
                "type": "object",
                "additionalProperties": true
            });
        }

        Self::kind_to_json_schema(&field.kind())
    }

    /// Convert Protobuf Kind to JSON Schema type
    fn kind_to_json_schema(kind: &prost_reflect::Kind) -> serde_json::Value {
        match kind {
            prost_reflect::Kind::Double | prost_reflect::Kind::Float => {
                serde_json::json!({"type": "number"})
            }
            prost_reflect::Kind::Int32
            | prost_reflect::Kind::Sint32
            | prost_reflect::Kind::Sfixed32
            | prost_reflect::Kind::Int64
            | prost_reflect::Kind::Sint64
            | prost_reflect::Kind::Sfixed64
            | prost_reflect::Kind::Uint32
            | prost_reflect::Kind::Fixed32
            | prost_reflect::Kind::Uint64
            | prost_reflect::Kind::Fixed64 => {
                serde_json::json!({"type": "integer"})
            }
            prost_reflect::Kind::Bool => serde_json::json!({"type": "boolean"}),
            prost_reflect::Kind::String => serde_json::json!({"type": "string"}),
            prost_reflect::Kind::Bytes => {
                serde_json::json!({"type": "string", "format": "byte"})
            }
            prost_reflect::Kind::Message(msg_desc) => {
                // Nested message -> recursively convert
                Self::message_descriptor_to_json_schema(msg_desc)
            }
            prost_reflect::Kind::Enum(enum_desc) => {
                // Enum -> string with enum values
                let enum_values: Vec<_> = enum_desc
                    .values()
                    .map(|v| serde_json::Value::String(v.name().to_string()))
                    .collect();
                serde_json::json!({
                    "type": "string",
                    "enum": enum_values
                })
            }
        }
    }
}

// create test
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use itertools::Itertools;
    use prost::Message;
    use prost_reflect::ReflectMessage;
    use std::io::{Cursor, Write};

    struct ProtobufDescriptorImpl {}
    impl ProtobufDescriptorLoader for ProtobufDescriptorImpl {}

    #[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, ::prost::Message)]
    pub struct TestArg {
        #[prost(string, repeated, tag = "1")]
        pub args: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    }

    #[test]
    fn test_load_protobuf_descriptor() -> Result<()> {
        let proto_string = r#"
        syntax = "proto3";

        package jobworkerp.data;

        message Job {
            string id = 1;
            string name = 2;
            string description = 3;
        }
        "#;
        let descriptor_pool =
            ProtobufDescriptorImpl::build_protobuf_descriptor(&proto_string.to_string())?;
        println!(
            "messages:{:?}",
            descriptor_pool.all_messages().collect_vec()
        );
        assert!(!descriptor_pool.all_messages().collect_vec().is_empty());
        let job_descriptor = descriptor_pool
            .get_message_by_name("jobworkerp.data.Job")
            .unwrap();
        job_descriptor
            .fields()
            .for_each(|field| println!("field:{field:?}"));
        assert_eq!(job_descriptor.full_name(), "jobworkerp.data.Job");
        assert_eq!(job_descriptor.package_name(), "jobworkerp.data");
        assert_eq!(job_descriptor.name(), "Job");
        Ok(())
    }

    #[test]
    fn test_read_by_protobuf_descriptor() -> Result<()> {
        let proto_string = r#"
syntax = "proto3";

// only for test
// job args
message TestArg {
  repeated string args = 1;
}
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let test_arg_descriptor = descriptor.get_message_by_name("TestArg").unwrap();
        assert_eq!(test_arg_descriptor.full_name(), "TestArg");
        assert_eq!(test_arg_descriptor.package_name(), "");
        assert_eq!(test_arg_descriptor.name(), "TestArg");
        let message = descriptor.get_message_by_name_from_bytes(
            "TestArg",
            TestArg {
                args: vec!["fuga".to_string(), "hoge".to_string()],
            }
            .encode_to_vec()
            .as_slice(),
        )?;
        assert_eq!(message.descriptor().name(), "TestArg");
        let args_field = message.get_field_by_name("args").unwrap();
        let args_list = args_field.as_list().unwrap();
        let args: Vec<&str> = args_list.iter().flat_map(|v| v.as_str()).collect_vec();
        assert_eq!(args, vec!["fuga", "hoge"]);

        Ok(())
    }

    #[test]
    fn test_get_message_from_json() -> Result<()> {
        let proto_string = r#"
        syntax = "proto3";

        package jobworkerp.data;

        message Job {
            int64 id = 1;
            string job_name = 2;
            string description = 3;
            repeated string tags = 4;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        assert_eq!(
            descriptor.get_message_names(),
            vec!["jobworkerp.data.Job".to_string()]
        );
        let json = r#"
        {
            "id": 1,
            "jobName": "test name",
            "description": "test desc:\n あいうえお",
            "tags": ["tag1", "tag2"]
        }
        "#;
        let message = descriptor.get_message_by_name_from_json("jobworkerp.data.Job", json)?;

        assert_eq!(message.descriptor().name(), "Job");
        assert_eq!(
            message.get_field_by_name("id").unwrap().as_i64().unwrap(),
            1
        );
        assert_eq!(
            message
                .get_field_by_name("job_name")
                .unwrap()
                .as_str()
                .unwrap(),
            "test name"
        );
        assert_eq!(
            message
                .get_field_by_name("description")
                .unwrap()
                .as_str()
                .unwrap(),
            "test desc:\n あいうえお"
        );
        let tags_field = message.get_field_by_name("tags").unwrap();
        let tags_list = tags_field.as_list().unwrap();
        let tags: Vec<&str> = tags_list.iter().flat_map(|v| v.as_str()).collect_vec();
        assert_eq!(tags, vec!["tag1", "tag2"]);

        ProtobufDescriptor::print_dynamic_message(&message, true);

        let bytes = message.encode_to_vec();
        let cursor = Cursor::new(bytes);
        let mes = DynamicMessage::decode(
            descriptor
                .get_message_by_name("jobworkerp.data.Job")
                .unwrap(),
            cursor,
        )?;
        println!("message:{mes:?}");
        std::io::stdout().flush()?;
        assert_eq!(message, mes);
        assert_eq!(
            ProtobufDescriptor::dynamic_message_to_string(&message, false),
            "id: 1\njob_name: test name\ndescription: test desc:\n あいうえお\ntags: [tag1, tag2]\n"
                .to_string()
        );
        let json = ProtobufDescriptor::message_to_json_value(&message)?;
        assert_eq!(
            json,
            serde_json::json!({
                "id": "1", // XXX string?
                "jobName": "test name",
                "description": "test desc:\n あいうえお",
                "tags": ["tag1", "tag2"]
            })
        );
        Ok(())
    }

    #[test]
    fn test_message_descriptor_to_json_schema_basic_types() -> Result<()> {
        let proto_string = r#"
        syntax = "proto3";

        message CommandArgs {
            string command = 1;
            repeated string args = 2;
            int32 timeout_ms = 3;
            bool verbose = 4;
            double priority = 5;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let msg_descriptor = descriptor.get_message_by_name("CommandArgs").unwrap();

        let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);

        // Verify structure
        assert_eq!(json_schema["type"], "object");
        assert!(json_schema["properties"].is_object());

        // Verify field types
        let props = json_schema["properties"].as_object().unwrap();
        assert_eq!(props["command"]["type"], "string");
        assert_eq!(props["args"]["type"], "array");
        assert_eq!(props["args"]["items"]["type"], "string");
        assert_eq!(props["timeoutMs"]["type"], "integer");
        assert_eq!(props["verbose"]["type"], "boolean");
        assert_eq!(props["priority"]["type"], "number");

        println!(
            "JSON Schema: {}",
            serde_json::to_string_pretty(&json_schema)?
        );
        Ok(())
    }

    #[test]
    fn test_message_descriptor_to_json_schema_nested_message() -> Result<()> {
        let proto_string = r#"
        syntax = "proto3";

        message Address {
            string street = 1;
            string city = 2;
        }

        message Person {
            string name = 1;
            int32 age = 2;
            Address address = 3;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let msg_descriptor = descriptor.get_message_by_name("Person").unwrap();

        let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);

        let props = json_schema["properties"].as_object().unwrap();
        assert_eq!(props["name"]["type"], "string");
        assert_eq!(props["age"]["type"], "integer");

        // Verify nested message
        let address_schema = &props["address"];
        assert_eq!(address_schema["type"], "object");
        let address_props = address_schema["properties"].as_object().unwrap();
        assert_eq!(address_props["street"]["type"], "string");
        assert_eq!(address_props["city"]["type"], "string");

        println!(
            "JSON Schema: {}",
            serde_json::to_string_pretty(&json_schema)?
        );
        Ok(())
    }

    #[test]
    fn test_message_descriptor_to_json_schema_enum() -> Result<()> {
        let proto_string = r#"
        syntax = "proto3";

        enum Status {
            UNKNOWN = 0;
            PENDING = 1;
            RUNNING = 2;
            COMPLETED = 3;
        }

        message Task {
            string name = 1;
            Status status = 2;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let msg_descriptor = descriptor.get_message_by_name("Task").unwrap();

        let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);

        let props = json_schema["properties"].as_object().unwrap();
        assert_eq!(props["name"]["type"], "string");

        // Verify enum field
        let status_schema = &props["status"];
        assert_eq!(status_schema["type"], "string");
        let enum_values = status_schema["enum"].as_array().unwrap();
        assert_eq!(enum_values.len(), 4);
        assert!(enum_values.contains(&serde_json::Value::String("UNKNOWN".to_string())));
        assert!(enum_values.contains(&serde_json::Value::String("PENDING".to_string())));
        assert!(enum_values.contains(&serde_json::Value::String("RUNNING".to_string())));
        assert!(enum_values.contains(&serde_json::Value::String("COMPLETED".to_string())));

        println!(
            "JSON Schema: {}",
            serde_json::to_string_pretty(&json_schema)?
        );
        Ok(())
    }

    #[test]
    fn test_message_descriptor_to_json_schema_map() -> Result<()> {
        let proto_string = r#"
        syntax = "proto3";

        message Config {
            string name = 1;
            map<string, string> labels = 2;
            map<string, int32> counters = 3;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let msg_descriptor = descriptor.get_message_by_name("Config").unwrap();

        let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);

        let props = json_schema["properties"].as_object().unwrap();
        assert_eq!(props["name"]["type"], "string");

        // Verify map<string, string>
        let labels_schema = &props["labels"];
        assert_eq!(labels_schema["type"], "object");
        assert_eq!(labels_schema["additionalProperties"]["type"], "string");

        // Verify map<string, int32>
        let counters_schema = &props["counters"];
        assert_eq!(counters_schema["type"], "object");
        assert_eq!(counters_schema["additionalProperties"]["type"], "integer");

        println!(
            "JSON Schema: {}",
            serde_json::to_string_pretty(&json_schema)?
        );
        Ok(())
    }

    #[test]
    fn test_message_descriptor_to_json_schema_deeply_nested_message() -> Result<()> {
        // Test case for 3+ levels of nested messages
        let proto_string = r#"
        syntax = "proto3";

        message GeoCoordinates {
            double latitude = 1;
            double longitude = 2;
        }

        message Location {
            string name = 1;
            GeoCoordinates coordinates = 2;
        }

        message Address {
            string street = 1;
            string city = 2;
            Location location = 3;
        }

        message Company {
            string name = 1;
            Address headquarters = 2;
        }

        message Person {
            string name = 1;
            int32 age = 2;
            Company employer = 3;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let msg_descriptor = descriptor.get_message_by_name("Person").unwrap();

        let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);

        // Level 1: Person
        let props = json_schema["properties"].as_object().unwrap();
        assert_eq!(props["name"]["type"], "string");
        assert_eq!(props["age"]["type"], "integer");

        // Level 2: Company (nested in Person)
        let company_schema = &props["employer"];
        assert_eq!(company_schema["type"], "object");
        let company_props = company_schema["properties"].as_object().unwrap();
        assert_eq!(company_props["name"]["type"], "string");

        // Level 3: Address (nested in Company)
        let address_schema = &company_props["headquarters"];
        assert_eq!(address_schema["type"], "object");
        let address_props = address_schema["properties"].as_object().unwrap();
        assert_eq!(address_props["street"]["type"], "string");
        assert_eq!(address_props["city"]["type"], "string");

        // Level 4: Location (nested in Address)
        let location_schema = &address_props["location"];
        assert_eq!(location_schema["type"], "object");
        let location_props = location_schema["properties"].as_object().unwrap();
        assert_eq!(location_props["name"]["type"], "string");

        // Level 5: GeoCoordinates (nested in Location)
        let coords_schema = &location_props["coordinates"];
        assert_eq!(coords_schema["type"], "object");
        let coords_props = coords_schema["properties"].as_object().unwrap();
        assert_eq!(coords_props["latitude"]["type"], "number");
        assert_eq!(coords_props["longitude"]["type"], "number");

        println!(
            "Deeply nested JSON Schema: {}",
            serde_json::to_string_pretty(&json_schema)?
        );
        Ok(())
    }

    #[test]
    fn test_message_descriptor_to_json_schema_nested_with_arrays() -> Result<()> {
        // Test case for nested messages with repeated fields (arrays)
        let proto_string = r#"
        syntax = "proto3";

        message Tag {
            string key = 1;
            string value = 2;
        }

        message Metadata {
            string description = 1;
            repeated Tag tags = 2;
        }

        message Item {
            string name = 1;
            Metadata metadata = 2;
        }

        message Category {
            string name = 1;
            repeated Item items = 2;
        }

        message Catalog {
            string title = 1;
            repeated Category categories = 2;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let msg_descriptor = descriptor.get_message_by_name("Catalog").unwrap();

        let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);

        // Level 1: Catalog
        let props = json_schema["properties"].as_object().unwrap();
        assert_eq!(props["title"]["type"], "string");

        // Level 2: Category array
        let categories_schema = &props["categories"];
        assert_eq!(categories_schema["type"], "array");
        let category_item = &categories_schema["items"];
        assert_eq!(category_item["type"], "object");
        let category_props = category_item["properties"].as_object().unwrap();
        assert_eq!(category_props["name"]["type"], "string");

        // Level 3: Item array (nested in Category)
        let items_schema = &category_props["items"];
        assert_eq!(items_schema["type"], "array");
        let item_schema = &items_schema["items"];
        assert_eq!(item_schema["type"], "object");
        let item_props = item_schema["properties"].as_object().unwrap();
        assert_eq!(item_props["name"]["type"], "string");

        // Level 4: Metadata (nested in Item)
        let metadata_schema = &item_props["metadata"];
        assert_eq!(metadata_schema["type"], "object");
        let metadata_props = metadata_schema["properties"].as_object().unwrap();
        assert_eq!(metadata_props["description"]["type"], "string");

        // Level 5: Tag array (nested in Metadata)
        let tags_schema = &metadata_props["tags"];
        assert_eq!(tags_schema["type"], "array");
        let tag_schema = &tags_schema["items"];
        assert_eq!(tag_schema["type"], "object");
        let tag_props = tag_schema["properties"].as_object().unwrap();
        assert_eq!(tag_props["key"]["type"], "string");
        assert_eq!(tag_props["value"]["type"], "string");

        println!(
            "Nested arrays JSON Schema: {}",
            serde_json::to_string_pretty(&json_schema)?
        );
        Ok(())
    }

    #[test]
    fn test_message_descriptor_to_json_schema_nested_with_maps() -> Result<()> {
        // Test case for nested messages with map fields
        let proto_string = r#"
        syntax = "proto3";

        message Attribute {
            string type = 1;
            string value = 2;
        }

        message Properties {
            map<string, string> labels = 1;
            map<string, Attribute> attributes = 2;
        }

        message Resource {
            string name = 1;
            Properties properties = 2;
        }

        message Namespace {
            string name = 1;
            map<string, Resource> resources = 2;
        }
        "#;
        let descriptor = ProtobufDescriptor::new(&proto_string.to_string())?;
        let msg_descriptor = descriptor.get_message_by_name("Namespace").unwrap();

        let json_schema = ProtobufDescriptor::message_descriptor_to_json_schema(&msg_descriptor);

        // Level 1: Namespace
        let props = json_schema["properties"].as_object().unwrap();
        assert_eq!(props["name"]["type"], "string");

        // Level 2: Resource map
        let resources_schema = &props["resources"];
        assert_eq!(resources_schema["type"], "object");
        let resource_schema = &resources_schema["additionalProperties"];
        assert_eq!(resource_schema["type"], "object");
        let resource_props = resource_schema["properties"].as_object().unwrap();
        assert_eq!(resource_props["name"]["type"], "string");

        // Level 3: Properties (nested in Resource)
        let properties_schema = &resource_props["properties"];
        assert_eq!(properties_schema["type"], "object");
        let properties_props = properties_schema["properties"].as_object().unwrap();

        // Level 4: labels map (map<string, string>)
        let labels_schema = &properties_props["labels"];
        assert_eq!(labels_schema["type"], "object");
        assert_eq!(labels_schema["additionalProperties"]["type"], "string");

        // Level 4: attributes map (map<string, Attribute>)
        let attributes_schema = &properties_props["attributes"];
        assert_eq!(attributes_schema["type"], "object");
        let attribute_schema = &attributes_schema["additionalProperties"];
        assert_eq!(attribute_schema["type"], "object");
        let attribute_props = attribute_schema["properties"].as_object().unwrap();
        assert_eq!(attribute_props["type"]["type"], "string");
        assert_eq!(attribute_props["value"]["type"], "string");

        println!(
            "Nested maps JSON Schema: {}",
            serde_json::to_string_pretty(&json_schema)?
        );
        Ok(())
    }
}
