use super::{Connack, Connect, Packet, Publish, Suback, Subscribe, Unsubscribe};
use super::{ConnectReturnCode, Error, SubscribeReturnCodes, SubscribeTopic};
use super::{Header, LastWill, PacketIdentifier, PacketType, Protocol, QoS, MULTIPLIER};
use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use std::sync::Arc;
use tokio_io::codec::{Decoder, Encoder};

#[derive(Default)]
pub struct MqttCodec {}

impl MqttCodec {
    pub fn new() -> Self {
        MqttCodec::default()
    }
}

impl Decoder for MqttCodec {
    type Item = Packet;
    type Error = super::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.read_packet(src)
    }
}

impl Encoder for MqttCodec {
    type Item = Packet;
    type Error = super::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.write_packet(dst, &item)
    }
}

impl MqttCodec {
    fn read_packet(&mut self, bytes: &BytesMut) -> Result<Option<Packet>, Error> {
        let (header, mut buf, len) = match (bytes.get(0), self.read_remaining_length(bytes)) {
            (Some(hd), Ok(Some((len_len, len)))) => {
                let header = Header::new(*hd, len)?;
                if len == 0 {
                    // no payload packets
                    return match header.typ {
                        PacketType::Pingreq => Ok(Some(Packet::Pingreq)),
                        PacketType::Pingresp => Ok(Some(Packet::Pingresp)),
                        _ => Err(Error::PayloadRequired),
                    };
                } else {
                    //Total packet length is header byte, plus variable length size, plus that size
                    if bytes.len() != (len_len + len + 1) {
                        return Ok(None);
                    };

                    Ok((header, bytes.into_buf(), len))
                }
            }
            (None, _) | (_, Ok(None)) => return Ok(None),
            (_, Err(e)) => Err(e),
        }?;

        match header.typ {
            PacketType::Connect => Ok(Some(Packet::Connect(self.read_connect(&mut buf, header)?))),
            PacketType::Connack => Ok(Some(Packet::Connack(self.read_connack(&mut buf, header)?))),
            PacketType::Publish => Ok(Some(Packet::Publish(self.read_publish(&mut buf, header)?))),
            PacketType::Puback => {
                if len != 2 {
                    return Err(Error::PayloadSizeIncorrect);
                }
                let pid = buf.get_u16_be();
                Ok(Some(Packet::Puback(PacketIdentifier(pid))))
            }
            PacketType::Pubrec => {
                if len != 2 {
                    return Err(Error::PayloadSizeIncorrect);
                }
                let pid = buf.get_u16_be();
                Ok(Some(Packet::Pubrec(PacketIdentifier(pid))))
            }
            PacketType::Pubrel => {
                if len != 2 {
                    return Err(Error::PayloadSizeIncorrect);
                }
                let pid = buf.get_u16_be();
                Ok(Some(Packet::Pubrel(PacketIdentifier(pid))))
            }
            PacketType::Pubcomp => {
                if len != 2 {
                    return Err(Error::PayloadSizeIncorrect);
                }
                let pid = buf.get_u16_be();
                Ok(Some(Packet::Pubcomp(PacketIdentifier(pid))))
            }
            PacketType::Subscribe => Ok(Some(Packet::Subscribe(
                self.read_subscribe(&mut buf, header)?,
            ))),
            PacketType::Suback => Ok(Some(Packet::Suback(self.read_suback(&mut buf, header)?))),
            PacketType::Unsubscribe => Ok(Some(Packet::Unsubscribe(
                self.read_unsubscribe(&mut buf, header)?,
            ))),
            PacketType::Unsuback => {
                if len != 2 {
                    return Err(Error::PayloadSizeIncorrect);
                }
                let pid = buf.get_u16_be();
                Ok(Some(Packet::Unsuback(PacketIdentifier(pid))))
            }
            PacketType::Pingreq => Err(Error::IncorrectPacketFormat),
            PacketType::Pingresp => Err(Error::IncorrectPacketFormat),
            _ => Err(Error::UnsupportedPacketType),
        }
    }

    fn read_connect(&mut self, buf: &mut Buf, _: Header) -> Result<Box<Connect>, Error> {
        let protocol_name = self.read_mqtt_string(buf)?;
        let protocol_level = buf.get_u8();
        let protocol = Protocol::new(protocol_name.as_ref(), protocol_level)?;

        let connect_flags = buf.get_u8();
        let keep_alive = buf.get_u16_be();
        let client_id = self.read_mqtt_string(buf)?;

        let last_will = match connect_flags & 0b100 {
            0 => {
                if (connect_flags & 0b00111000) != 0 {
                    return Err(Error::IncorrectPacketFormat);
                }
                None
            }
            _ => {
                let will_topic = self.read_mqtt_string(buf)?;
                let will_message = self.read_mqtt_string(buf)?;
                let will_qod = QoS::from_u8((connect_flags & 0b11000) >> 3)?;
                Some(LastWill {
                    topic: will_topic,
                    message: will_message,
                    qos: will_qod,
                    retain: (connect_flags & 0b00100000) != 0,
                })
            }
        };

        let username = match connect_flags & 0b10000000 {
            0 => None,
            _ => Some(self.read_mqtt_string(buf)?),
        };

        let password = match connect_flags & 0b01000000 {
            0 => None,
            _ => Some(self.read_mqtt_string(buf)?),
        };

        Ok(Box::new(Connect {
            protocol: protocol,
            keep_alive: keep_alive,
            client_id: client_id,
            clean_session: (connect_flags & 0b10) != 0,
            last_will: last_will,
            username: username,
            password: password,
        }))
    }

    fn read_connack(&mut self, buf: &mut Buf, header: Header) -> Result<Connack, Error> {
        if header.len != 2 {
            return Err(Error::PayloadSizeIncorrect);
        }
        let flags = buf.get_u8();
        let return_code = buf.get_u8();
        Ok(Connack {
            session_present: (flags & 0x01) == 1,
            code: ConnectReturnCode::from_u8(return_code)?,
        })
    }

    fn read_publish(&mut self, buf: &mut Buf, header: Header) -> Result<Box<Publish>, Error> {
        let topic_name = self.read_mqtt_string(buf);
        // Packet identifier exists where QoS > 0
        let pid = if header.qos().unwrap() != QoS::AtMostOnce {
            Some(PacketIdentifier(buf.get_u16_be()))
        } else {
            None
        };
        let mut payload = Vec::new();
        buf.copy_to_slice(&mut payload);

        Ok(Box::new(Publish {
            dup: header.dup(),
            qos: header.qos()?,
            retain: header.retain(),
            topic_name: topic_name?,
            pid: pid,
            payload: Arc::new(payload),
        }))
    }

    fn read_subscribe(&mut self, buf: &mut Buf, header: Header) -> Result<Box<Subscribe>, Error> {
        let pid = buf.get_u16_be();
        let mut remaining_bytes = header.len - 2;
        let mut topics = Vec::with_capacity(1);

        while remaining_bytes > 0 {
            let topic_filter = self.read_mqtt_string(buf)?;
            let requested_qod = buf.get_u8();
            remaining_bytes -= topic_filter.len() + 3;
            topics.push(SubscribeTopic {
                topic_path: topic_filter,
                qos: QoS::from_u8(requested_qod)?,
            });
        }

        Ok(Box::new(Subscribe {
            pid: PacketIdentifier(pid),
            topics: topics,
        }))
    }

    fn read_suback(&mut self, buf: &mut Buf, header: Header) -> Result<Box<Suback>, Error> {
        let pid = buf.get_u16_be();
        let mut remaining_bytes = header.len - 2;
        let mut return_codes = Vec::with_capacity(remaining_bytes);

        while remaining_bytes > 0 {
            let return_code = buf.get_u8();
            if return_code >> 7 == 1 {
                return_codes.push(SubscribeReturnCodes::Failure)
            } else {
                return_codes.push(SubscribeReturnCodes::Success(QoS::from_u8(
                    return_code & 0x3,
                )?));
            }
            remaining_bytes -= 1
        }

        Ok(Box::new(Suback {
            pid: PacketIdentifier(pid),
            return_codes: return_codes,
        }))
    }

    fn read_unsubscribe(
        &mut self,
        buf: &mut Buf,
        header: Header,
    ) -> Result<Box<Unsubscribe>, Error> {
        let pid = buf.get_u16_be();
        let mut remaining_bytes = header.len - 2;
        let mut topics = Vec::with_capacity(1);

        while remaining_bytes > 0 {
            let topic_filter = self.read_mqtt_string(buf)?;
            remaining_bytes -= topic_filter.len() + 2;
            topics.push(topic_filter);
        }

        Ok(Box::new(Unsubscribe {
            pid: PacketIdentifier(pid),
            topics: topics,
        }))
    }

    fn read_mqtt_string(&mut self, buf: &mut Buf) -> Result<String, Error> {
        let len = buf.get_u16_be() as usize;
        let mut data = Vec::with_capacity(len);
        buf.copy_to_slice(&mut data);
        Ok(String::from_utf8_lossy(&data).to_string())
    }

    fn read_remaining_length(&mut self, bytes: &BytesMut) -> Result<Option<(usize, usize)>, Error> {
        let mut index: usize = 1;
        let mut mult: usize = 1;
        let mut len: usize = 0;
        let mut done = false;

        while !done {
            match bytes.get(index) {
                Some(byte) => {
                    index += 1usize;
                    len += ((byte & 0x7F) as usize) * mult;
                    mult *= 0x80;
                    if mult > MULTIPLIER {
                        return Err(Error::MalformedRemainingLength);
                    }
                    done = (byte & 0x80) == 0
                }
                None => return Ok(None),
            }
        }

        Ok(Some((index - 1usize, len)))
    }

    fn write_packet(&mut self, buf: &mut BufMut, packet: &Packet) -> Result<(), Error> {
        match packet {
            &Packet::Connect(ref connect) => {
                buf.put_u8(0b00010000);
                let prot_name = connect.protocol.name();
                let mut len = 8 + prot_name.len() + connect.client_id.len();
                if let Some(ref last_will) = connect.last_will {
                    len += 4 + last_will.topic.len() + last_will.message.len();
                }
                if let Some(ref username) = connect.username {
                    len += 2 + username.len();
                }
                if let Some(ref password) = connect.password {
                    len += 2 + password.len();
                }
                self.write_remaining_length(buf, len);
                self.write_mqtt_string(buf, prot_name);
                buf.put_u8(connect.protocol.level());
                let mut connect_flags = 0;
                if connect.clean_session {
                    connect_flags |= 0x02;
                }
                if let Some(ref last_will) = connect.last_will {
                    connect_flags |= 0x04;
                    connect_flags |= last_will.qos.to_u8() << 3;
                    if last_will.retain {
                        connect_flags |= 0x20;
                    }
                }
                if let Some(_) = connect.password {
                    connect_flags |= 0x40;
                }
                if let Some(_) = connect.username {
                    connect_flags |= 0x80;
                }
                buf.put_u8(connect_flags);
                buf.put_u16_be(connect.keep_alive);
                self.write_mqtt_string(buf, connect.client_id.as_ref());
                if let Some(ref last_will) = connect.last_will {
                    self.write_mqtt_string(buf, last_will.topic.as_ref());
                    self.write_mqtt_string(buf, last_will.message.as_ref());
                }
                if let Some(ref username) = connect.username {
                    self.write_mqtt_string(buf, username);
                }
                if let Some(ref password) = connect.password {
                    self.write_mqtt_string(buf, password);
                }
                Ok(())
            }
            &Packet::Connack(ref connack) => {
                buf.put_slice(&[
                    0x20,
                    0x02,
                    connack.session_present as u8,
                    connack.code.to_u8(),
                ]);
                Ok(())
            }
            &Packet::Publish(ref publish) => {
                buf.put_u8(
                    0b00110000
                        | publish.retain as u8
                        | (publish.qos.to_u8() << 1)
                        | ((publish.dup as u8) << 3),
                );
                let mut len = publish.topic_name.len() + 2 + publish.payload.len();
                if publish.qos != QoS::AtMostOnce && None != publish.pid {
                    len += 2;
                }
                self.write_remaining_length(buf, len);
                self.write_mqtt_string(buf, publish.topic_name.as_str());
                if publish.qos != QoS::AtMostOnce {
                    if let Some(pid) = publish.pid {
                        buf.put_u16_be(pid.0);
                    }
                }
                buf.put_slice(&publish.payload.as_ref());
                Ok(())
            }
            &Packet::Puback(ref pid) => {
                buf.put_slice(&[0x40, 0x02]);
                buf.put_u16_be(pid.0);
                Ok(())
            }
            &Packet::Pubrec(ref pid) => {
                buf.put_slice(&[0x50, 0x02]);
                buf.put_u16_be(pid.0);
                Ok(())
            }
            &Packet::Pubrel(ref pid) => {
                buf.put_slice(&[0x62, 0x02]);
                buf.put_u16_be(pid.0);
                Ok(())
            }
            &Packet::Pubcomp(ref pid) => {
                buf.put_slice(&[0x70, 0x02]);
                buf.put_u16_be(pid.0);
                Ok(())
            }
            &Packet::Subscribe(ref subscribe) => {
                buf.put_slice(&[0x82]);
                let len = 2 + subscribe
                    .topics
                    .iter()
                    .fold(0, |s, ref t| s + t.topic_path.len() + 3);
                self.write_remaining_length(buf, len);
                buf.put_u16_be(subscribe.pid.0);
                for topic in subscribe.topics.as_ref() as &Vec<SubscribeTopic> {
                    self.write_mqtt_string(buf, topic.topic_path.as_str());
                    buf.put_u8(topic.qos.to_u8());
                }
                Ok(())
            }
            &Packet::Suback(ref suback) => {
                buf.put_slice(&[0x90]);
                self.write_remaining_length(buf, suback.return_codes.len() + 2);
                buf.put_u16_be(suback.pid.0);
                let payload: Vec<u8> = suback
                    .return_codes
                    .iter()
                    .map({
                        |&code| match code {
                            SubscribeReturnCodes::Success(qos) => qos.to_u8(),
                            SubscribeReturnCodes::Failure => 0x80,
                        }
                    })
                    .collect();
                buf.put_slice(&payload);
                Ok(())
            }
            &Packet::Unsubscribe(ref unsubscribe) => {
                buf.put_slice(&[0xA2]);
                let len = 2 + unsubscribe
                    .topics
                    .iter()
                    .fold(0, |s, ref topic| s + topic.len() + 2);
                self.write_remaining_length(buf, len);
                buf.put_u16_be(unsubscribe.pid.0);
                for topic in unsubscribe.topics.as_ref() as &Vec<String> {
                    self.write_mqtt_string(buf, topic.as_str());
                }
                Ok(())
            }
            &Packet::Unsuback(ref pid) => {
                buf.put_slice(&[0xB0, 0x02]);
                buf.put_u16_be(pid.0);
                Ok(())
            }
            &Packet::Pingreq => {
                buf.put_slice(&[0xc0, 0]);
                Ok(())
            }
            &Packet::Pingresp => {
                buf.put_slice(&[0xd0, 0]);
                Ok(())
            }
            &Packet::Disconnect => {
                buf.put_slice(&[0xe0, 0]);
                Ok(())
            }
        }
    }

    fn write_mqtt_string(&mut self, buf: &mut BufMut, string: &str) {
        buf.put_u16_be(string.len() as u16);
        buf.put_slice(string.as_bytes());
    }

    fn write_remaining_length(&mut self, buf: &mut BufMut, len: usize) {
        let mut done = false;
        let mut x = len;

        while !done {
            let mut byte = (x % 128) as u8;
            x = x / 128;
            if x > 0 {
                byte = byte | 128;
            }
            buf.put_u8(byte);
            done = x <= 0;
        }
    }
}

#[cfg(test)]
mod test {
    use super::{
        ConnectReturnCode, LastWill, MqttCodec, PacketIdentifier, Protocol, QoS,
        SubscribeReturnCodes, SubscribeTopic,
    };
    use crate::protocol::packet::{
        Connack, Connect, Packet, Publish, Suback, Subscribe, Unsubscribe,
    };
    use bytes::BytesMut;
    use std::io::Cursor;
    use std::sync::Arc;
    use tokio_codec::{Decoder, Encoder};

    #[test]
    fn read_packet_connect_mqtt_protocol_test() {
        let mut stream = BytesMut::from(vec![
            0x10, 39, 0x00, 0x04, 'M' as u8, 'Q' as u8, 'T' as u8, 'T' as u8, 0x04,
            0b11001110, // +username, +password, -will retain, will qos=1, +last_will, +clean_session
            0x00, 0x0a, // 10 sec
            0x00, 0x04, 't' as u8, 'e' as u8, 's' as u8, 't' as u8, // client_id
            0x00, 0x02, '/' as u8, 'a' as u8, // will topic = '/a'
            0x00, 0x07, 'o' as u8, 'f' as u8, 'f' as u8, 'l' as u8, 'i' as u8, 'n' as u8,
            'e' as u8, // will msg = 'offline'
            0x00, 0x04, 'r' as u8, 'u' as u8, 's' as u8, 't' as u8, // username = 'rust'
            0x00, 0x02, 'm' as u8, 'q' as u8, // password = 'mq'
        ]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Connect(Box::new(Connect {
                protocol: Protocol::MQTT(4),
                keep_alive: 10,
                client_id: "test".to_owned(),
                clean_session: true,
                last_will: Some(LastWill {
                    topic: "/a".to_owned(),
                    message: "offline".to_owned(),
                    retain: false,
                    qos: QoS::AtLeastOnce
                }),
                username: Some("rust".to_owned()),
                password: Some("mq".to_owned())
            }))
        );
    }

    #[test]
    fn read_packet_connect_mqisdp_protocol_test() {
        let mut bytes = BytesMut::from(vec![
            0x10, 18, 0x00, 0x06, 'M' as u8, 'Q' as u8, 'I' as u8, 's' as u8, 'd' as u8, 'p' as u8,
            0x03,
            0b00000000, // -username, -password, -will retain, will qos=0, -last_will, -clean_session
            0x00, 0x3c, // 60 sec
            0x00, 0x04, 't' as u8, 'e' as u8, 's' as u8, 't' as u8, // client_id
        ]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Connect(Box::new(Connect {
                protocol: Protocol::MQIsdp(3),
                keep_alive: 60,
                client_id: "test".to_owned(),
                clean_session: false,
                last_will: None,
                username: None,
                password: None
            }))
        );
    }

    #[test]
    fn read_packet_connack_test() {
        let mut stream = BytesMut::from(vec![0b00100000, 0x02, 0x01, 0x00]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Connack(Connack {
                session_present: true,
                code: ConnectReturnCode::Accepted
            })
        );
    }

    #[test]
    fn read_packet_publish_qos1_test() {
        let mut stream = BytesMut::from(vec![
            0b00110010, 11, 0x00, 0x03, 'a' as u8, '/' as u8, 'b' as u8, // topic name = 'a/b'
            0x00, 0x0a, // pid = 10
            0xF1, 0xF2, 0xF3, 0xF4,
        ]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Publish(Box::new(Publish {
                dup: false,
                qos: QoS::AtLeastOnce,
                retain: false,
                topic_name: "a/b".to_owned(),
                pid: Some(PacketIdentifier(10)),
                payload: Arc::new(vec![0xF1, 0xF2, 0xF3, 0xF4])
            }))
        );
    }

    #[test]
    fn read_packet_publish_qos0_test() {
        let mut stream = BytesMut::from(vec![
            0b00110000, 7, 0x00, 0x03, 'a' as u8, '/' as u8, 'b' as u8, // topic name = 'a/b'
            0x01, 0x02,
        ]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Publish(Box::new(Publish {
                dup: false,
                qos: QoS::AtMostOnce,
                retain: false,
                topic_name: "a/b".to_owned(),
                pid: None,
                payload: Arc::new(vec![0x01, 0x02])
            }))
        );
    }

    #[test]
    fn read_packet_puback_test() {
        let mut stream = BytesMut::from(vec![0b01000000, 0x02, 0x00, 0x0A]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();
        assert_eq!(packet, Packet::Puback(PacketIdentifier(10)));
    }

    #[test]
    fn read_packet_subscribe_test() {
        let mut stream = BytesMut::from(vec![
            0b10000010, 20, 0x01, 0x04, // pid = 260
            0x00, 0x03, 'a' as u8, '/' as u8, '+' as u8, // topic filter = 'a/+'
            0x00,      // qos = 0
            0x00, 0x01, '#' as u8, // topic filter = '#'
            0x01,      // qos = 1
            0x00, 0x05, 'a' as u8, '/' as u8, 'b' as u8, '/' as u8,
            'c' as u8, // topic filter = 'a/b/c'
            0x02,      // qos = 2
        ]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Subscribe(Box::new(Subscribe {
                pid: PacketIdentifier(260),
                topics: vec![
                    SubscribeTopic {
                        topic_path: "a/+".to_owned(),
                        qos: QoS::AtMostOnce
                    },
                    SubscribeTopic {
                        topic_path: "#".to_owned(),
                        qos: QoS::AtLeastOnce
                    },
                    SubscribeTopic {
                        topic_path: "a/b/c".to_owned(),
                        qos: QoS::ExactlyOnce
                    }
                ]
            }))
        );
    }

    #[test]
    fn read_packet_unsubscribe_test() {
        let mut stream = BytesMut::from(vec![
            0b10100010, 17, 0x00, 0x0F, // pid = 15
            0x00, 0x03, 'a' as u8, '/' as u8, '+' as u8, // topic filter = 'a/+'
            0x00, 0x01, '#' as u8, // topic filter = '#'
            0x00, 0x05, 'a' as u8, '/' as u8, 'b' as u8, '/' as u8,
            'c' as u8, // topic filter = 'a/b/c'
        ]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Unsubscribe(Box::new(Unsubscribe {
                pid: PacketIdentifier(15),
                topics: vec!["a/+".to_owned(), "#".to_owned(), "a/b/c".to_owned()]
            }))
        );
    }

    #[test]
    fn read_packet_suback_test() {
        let mut stream = BytesMut::from(vec![
            0x90, 4, 0x00, 0x0F, // pid = 15
            0x01, 0x80,
        ]);

        let packet = MqttCodec::new().decode(&mut stream).unwrap().unwrap();

        assert_eq!(
            packet,
            Packet::Suback(Box::new(Suback {
                pid: PacketIdentifier(15),
                return_codes: vec![
                    SubscribeReturnCodes::Success(QoS::AtLeastOnce),
                    SubscribeReturnCodes::Failure
                ]
            }))
        );
    }

    #[test]
    fn write_packet_connect_mqtt_protocol_test() {
        let connect = Packet::Connect(Box::new(Connect {
            protocol: Protocol::MQTT(4),
            keep_alive: 10,
            client_id: "test".to_owned(),
            clean_session: true,
            last_will: Some(LastWill {
                topic: "/a".to_owned(),
                message: "offline".to_owned(),
                retain: false,
                qos: QoS::AtLeastOnce,
            }),
            username: Some("rust".to_owned()),
            password: Some("mq".to_owned()),
        }));

        let mut vec = vec![];
        let mut stream = BytesMut::from(&mut vec);
        MqttCodec::new().encode(&connect,&mut stream).unwrap();

        assert_eq!(
            vec,
            vec![
                0x10, 39, 0x00, 0x04, 'M' as u8, 'Q' as u8, 'T' as u8, 'T' as u8, 0x04,
                0b11001110, // +username, +password, -will retain, will qos=1, +last_will, +clean_session
                0x00, 0x0a, // 10 sec
                0x00, 0x04, 't' as u8, 'e' as u8, 's' as u8, 't' as u8, // client_id
                0x00, 0x02, '/' as u8, 'a' as u8, // will topic = '/a'
                0x00, 0x07, 'o' as u8, 'f' as u8, 'f' as u8, 'l' as u8, 'i' as u8, 'n' as u8,
                'e' as u8, // will msg = 'offline'
                0x00, 0x04, 'r' as u8, 'u' as u8, 's' as u8, 't' as u8, // username = 'rust'
                0x00, 0x02, 'm' as u8, 'q' as u8 // password = 'mq'
            ]
        );
    }

    #[test]
    fn write_packet_connect_mqisdp_protocol_test() {
        let connect = Packet::Connect(Box::new(Connect {
            protocol: Protocol::MQIsdp(3),
            keep_alive: 60,
            client_id: "test".to_owned(),
            clean_session: false,
            last_will: None,
            username: None,
            password: None,
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&connect).unwrap();

        assert_eq!(
            stream.get_ref().clone(),
            vec![
                0x10, 18, 0x00, 0x06, 'M' as u8, 'Q' as u8, 'I' as u8, 's' as u8, 'd' as u8,
                'p' as u8, 0x03,
                0b00000000, // -username, -password, -will retain, will qos=0, -last_will, -clean_session
                0x00, 0x3c, // 60 sec
                0x00, 0x04, 't' as u8, 'e' as u8, 's' as u8, 't' as u8 // client_id
            ]
        );
    }

    #[test]
    fn write_packet_connack_test() {
        let connack = Packet::Connack(Connack {
            session_present: true,
            code: ConnectReturnCode::Accepted,
        });

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&connack).unwrap();

        assert_eq!(stream.get_ref().clone(), vec![0b00100000, 0x02, 0x01, 0x00]);
    }

    #[test]
    fn write_packet_publish_at_least_once_test() {
        let publish = Packet::Publish(Box::new(Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic_name: "a/b".to_owned(),
            pid: Some(PacketIdentifier(10)),
            payload: Arc::new(vec![0xF1, 0xF2, 0xF3, 0xF4]),
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&publish).unwrap();

        assert_eq!(
            stream.get_ref().clone(),
            vec![
                0b00110010, 11, 0x00, 0x03, 'a' as u8, '/' as u8, 'b' as u8, 0x00, 0x0a, 0xF1,
                0xF2, 0xF3, 0xF4
            ]
        );
    }

    #[test]
    fn write_packet_publish_at_most_once_test() {
        let publish = Packet::Publish(Box::new(Publish {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            topic_name: "a/b".to_owned(),
            pid: None,
            payload: Arc::new(vec![0xE1, 0xE2, 0xE3, 0xE4]),
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&publish).unwrap();

        assert_eq!(
            stream.get_ref().clone(),
            vec![
                0b00110000, 9, 0x00, 0x03, 'a' as u8, '/' as u8, 'b' as u8, 0xE1, 0xE2, 0xE3, 0xE4
            ]
        );
    }

    #[test]
    fn write_packet_subscribe_test() {
        let subscribe = Packet::Subscribe(Box::new(Subscribe {
            pid: PacketIdentifier(260),
            topics: vec![
                SubscribeTopic {
                    topic_path: "a/+".to_owned(),
                    qos: QoS::AtMostOnce,
                },
                SubscribeTopic {
                    topic_path: "#".to_owned(),
                    qos: QoS::AtLeastOnce,
                },
                SubscribeTopic {
                    topic_path: "a/b/c".to_owned(),
                    qos: QoS::ExactlyOnce,
                },
            ],
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&subscribe).unwrap();

        assert_eq!(
            stream.get_ref().clone(),
            vec![
                0b10000010, 20, 0x01, 0x04, // pid = 260
                0x00, 0x03, 'a' as u8, '/' as u8, '+' as u8, // topic filter = 'a/+'
                0x00,      // qos = 0
                0x00, 0x01, '#' as u8, // topic filter = '#'
                0x01,      // qos = 1
                0x00, 0x05, 'a' as u8, '/' as u8, 'b' as u8, '/' as u8,
                'c' as u8, // topic filter = 'a/b/c'
                0x02       // qos = 2
            ]
        );
    }
}
