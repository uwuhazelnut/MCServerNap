use crate::{config::Config, write_varint};
use serde_json::{Value, json};

pub struct PreserializedPackets {
    pub starting_message_packet: Vec<u8>,
    pub motd_packet: Vec<u8>,
}

impl PreserializedPackets {
    pub fn new(config: &Config) -> Self {
        let starting_message_packet = Self::serialize_starting_message(&config);
        let motd_packet = Self::serialize_motd(&config);

        PreserializedPackets {
            starting_message_packet,
            motd_packet,
        }
    }

    fn serialize_starting_message(config: &Config) -> Vec<u8> {
        let json_msg = json!({
            "text": config.connection_msg_text,
            "color": config.connection_msg_color,
            "bold": config.connection_msg_bold
        })
        .to_string();
        let mut packet_data = Vec::new();

        //Packet ID 0x00 (login disconnect)
        write_varint(0, &mut packet_data);

        write_varint(json_msg.len() as i32, &mut packet_data);
        packet_data.extend_from_slice(json_msg.as_bytes());

        let mut packet = Vec::new();
        write_varint(packet_data.len() as i32, &mut packet);
        packet.extend_from_slice(&packet_data);

        packet
    }

    fn serialize_motd(config: &Config) -> Vec<u8> {
        // Create custom MOTD JSON
        // Protocol is "an integer used to check for incompatibilities between the player's client and the server
        // they are trying to connect to.". 766 = Minecraft 1.20.5 (https://minecraft.fandom.com/wiki/Protocol_version)
        let mut motd_json_obj = json!({
            "version": {
                "name": "MCServerNap (1.20.5)",
                "protocol": 766
            },
            "players": {
                "max": 0,
                "online": 0,
                "sample": []
            },
            "description": {
                "text": config.motd_text,
                "color": config.motd_color,
                "bold": config.motd_bold
            }
        });

        if let Some(server_icon_base64) = config.server_icon.as_ref() {
            if let Value::Object(ref mut map) = motd_json_obj {
                map.insert(
                    "favicon".to_string(),
                    Value::String(format!("data:image/png;base64,{}", server_icon_base64)),
                );
            }
        }

        let motd_json = motd_json_obj.to_string();

        // Create status response packet
        let mut data = Vec::new();
        // Packet ID = 0 (status response)
        write_varint(0, &mut data);
        write_varint(motd_json.len() as i32, &mut data);
        data.extend_from_slice(motd_json.as_bytes());

        let mut packet = Vec::new();
        write_varint(data.len() as i32, &mut packet);
        packet.extend_from_slice(&data);

        packet
    }
}
