use anyhow::{Context, Result};
use itertools::Itertools;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, ReflectMessage};
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
        tonic_build::configure()
            // only output message descriptor
            .build_server(false)
            .build_client(false)
            .build_transport(false)
            .out_dir(&tempdir)
            .protoc_arg("--experimental_allow_proto3_optional")
            .file_descriptor_set_path(&descriptor_file) // for reflection
            .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
            .compile_protos(&[&tempfile], &[&tempdir])
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
        let temp_dir = tempfile::tempdir()
            .context(format!("on creating tempfile for proto: {}", proto_name))?;
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
            .ok_or(anyhow::anyhow!(
                "message not found by name: {}",
                message_name
            ))?;
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
            .ok_or(anyhow::anyhow!(
                "message not found by name: {}",
                message_name
            ))?;
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
    pub fn print_dynamic_message(message: &DynamicMessage, byte_to_string: bool) {
        let message_str = Self::dynamic_message_to_string(message, byte_to_string);
        println!("{}", message_str);
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
            prost_reflect::Value::Bool(v) => format!("{}", v),
            prost_reflect::Value::I32(v) => format!("{}", v),
            prost_reflect::Value::I64(v) => format!("{}", v),
            prost_reflect::Value::U32(v) => format!("{}", v),
            prost_reflect::Value::U64(v) => format!("{}", v),
            prost_reflect::Value::F32(v) => format!("{}", v),
            prost_reflect::Value::F64(v) => format!("{}", v),
            prost_reflect::Value::String(v) => v.to_string(),
            prost_reflect::Value::Bytes(v) => {
                if byte_to_string {
                    format!("{}", String::from_utf8_lossy(v))
                } else {
                    format!("{:x?}", v)
                }
            }
            prost_reflect::Value::EnumNumber(v) => format!("{:?}[enum]", v),
            prost_reflect::Value::Message(v) => Self::dynamic_message_to_string(v, byte_to_string),
            prost_reflect::Value::List(v) => {
                let list_str = v
                    .iter()
                    .map(|v| Self::value_to_string(v, byte_to_string))
                    .join(", ");
                format!("[{}]", list_str)
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
                format!("{{{}}}", map_str)
            }
        }
    }
    fn map_key_to_string(k: &prost_reflect::MapKey) -> String {
        match k {
            prost_reflect::MapKey::Bool(v) => format!("{}", v),
            prost_reflect::MapKey::I32(v) => format!("{}", v),
            prost_reflect::MapKey::I64(v) => format!("{}", v),
            prost_reflect::MapKey::U32(v) => format!("{}", v),
            prost_reflect::MapKey::U64(v) => format!("{}", v),
            prost_reflect::MapKey::String(v) => v.to_string(),
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
            .for_each(|field| println!("field:{:?}", field));
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
            string name = 2;
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
            "name": "test name",
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
            message.get_field_by_name("name").unwrap().as_str().unwrap(),
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
        println!("message:{:?}", mes);
        std::io::stdout().flush()?;
        assert_eq!(message, mes);
        assert_eq!(
            ProtobufDescriptor::dynamic_message_to_string(&message, false),
            "id: 1\nname: test name\ndescription: test desc:\n あいうえお\ntags: [tag1, tag2]\n"
                .to_string()
        );
        Ok(())
    }
}
