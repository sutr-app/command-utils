use anyhow::{anyhow, Result};
use encoding::label::encoding_from_whatwg_label;
use encoding::DecoderTrap;

/// read to end and detect char-encoding and decode to utf-8
/// (ignore unknown character)
/// ref. https://github.com/thuleqaid/rust-chardet
pub fn encode_to_utf8<R>(input: &mut R) -> Result<String>
where
    R: std::io::Read,
{
    let mut reader: Vec<u8> = Vec::new();

    // read file
    input
        .read_to_end(&mut reader)
        .map_err(|e| anyhow!("Could not read file: {}", e))?;

    encode_to_utf8_raw(&reader)
}

pub fn encode_to_utf8_raw(input: &[u8]) -> Result<String> {
    // detect charset of the file
    let result = chardet::detect(input);
    // result.0 Encode
    // result.1 Confidence
    // result.2 Language

    // decode file into utf-8
    let coder = encoding_from_whatwg_label(chardet::charset2encoding(&result.0));
    if let Some(c) = coder {
        c.decode(input, DecoderTrap::Ignore)
            .map_err(|e| anyhow!("Error:{:?}", e))
    } else {
        Err(anyhow!("cannot find character encodings: {:?}", &result))
    }
}
