//! Basically a copy of `mqttbytes`'s publish packet parsing, just returns parsed publish as a
//! wrapped around `Bytes` for cheap cloning across threads.

use bytes::{Bytes, BytesMut};

#[derive(Debug, Clone)]
pub enum MQTTRead {
    Publish(Bytes),
    Disconnect,
}

impl MQTTRead {
    pub fn read(stream: &mut BytesMut) -> Result<MQTTRead, mqttbytes::Error> {
        let mut iter = stream.iter();
        let stream_len = iter.len();
        if stream_len < 2 {
            return Err(mqttbytes::Error::InsufficientBytes(2 - stream_len));
        }

        let byte1 = *iter.next().unwrap();
        let (len_len, remaining_len) = length(iter)?;
        let fixed_header_len = len_len + 1;

        let packet = stream.split_to(fixed_header_len + remaining_len).freeze();

        if packet_type(byte1)? == mqttbytes::PacketType::Disconnect {
            return Ok(MQTTRead::Disconnect);
        }

        let qos = mqttbytes::qos((byte1 & 0b0110) >> 1)?;
        if qos != mqttbytes::QoS::AtMostOnce {
            return Err(mqttbytes::Error::InvalidQoS(0));
        }

        let end = fixed_header_len
            + u16::from_be_bytes([packet[fixed_header_len], packet[fixed_header_len + 1]]) as usize;
        if std::str::from_utf8(&packet[fixed_header_len..end]).is_err() {
            return Err(mqttbytes::Error::TopicNotUtf8);
        }

        Ok(MQTTRead::Publish(packet))
    }
}

pub fn length(stream: std::slice::Iter<u8>) -> Result<(usize, usize), mqttbytes::Error> {
    let mut len: usize = 0;
    let mut len_len = 0;
    let mut done = false;
    let mut shift = 0;

    // Use continuation bit at position 7 to continue reading next
    // byte to frame 'length'.
    // Stream 0b1xxx_xxxx 0b1yyy_yyyy 0b1zzz_zzzz 0b0www_wwww will
    // be framed as number 0bwww_wwww_zzz_zzzz_yyy_yyyy_xxx_xxxx
    for byte in stream {
        len_len += 1;
        let byte = *byte as usize;
        len += (byte & 0x7F) << shift;

        // stop when continue bit is 0
        done = (byte & 0x80) == 0;
        if done {
            break;
        }

        shift += 7;

        // Only a max of 4 bytes allowed for remaining length
        // more than 4 shifts (0, 7, 14, 21) implies bad length
        if shift > 21 {
            return Err(mqttbytes::Error::MalformedRemainingLength);
        }
    }

    // Not enough bytes to frame remaining length. wait for
    // one more byte
    if !done {
        return Err(mqttbytes::Error::InsufficientBytes(1));
    }

    Ok((len_len, len))
}

fn packet_type(byte1: u8) -> Result<mqttbytes::PacketType, mqttbytes::Error> {
    let num = byte1 >> 4;
    match num {
        1 => Ok(mqttbytes::PacketType::Connect),
        2 => Ok(mqttbytes::PacketType::ConnAck),
        3 => Ok(mqttbytes::PacketType::Publish),
        4 => Ok(mqttbytes::PacketType::PubAck),
        5 => Ok(mqttbytes::PacketType::PubRec),
        6 => Ok(mqttbytes::PacketType::PubRel),
        7 => Ok(mqttbytes::PacketType::PubComp),
        8 => Ok(mqttbytes::PacketType::Subscribe),
        9 => Ok(mqttbytes::PacketType::SubAck),
        10 => Ok(mqttbytes::PacketType::Unsubscribe),
        11 => Ok(mqttbytes::PacketType::UnsubAck),
        12 => Ok(mqttbytes::PacketType::PingReq),
        13 => Ok(mqttbytes::PacketType::PingResp),
        14 => Ok(mqttbytes::PacketType::Disconnect),
        _ => Err(mqttbytes::Error::InvalidPacketType(num)),
    }
}
