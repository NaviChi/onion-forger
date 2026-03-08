use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::Value;

fn main() {
    let token = "eyJhbGciOiJSUzUxMiIsInR5cCI6IkpXVCJ9.eyJjb2xvcl9pbnB1dF9iYWNrZ3JvdW5kIjoiIzJCMkEzM0ZGIiwiY29sb3JfbWFpbiI6IiMyMjIyMjJGRiIsImNvbG9yX21haW5fZGFyayI6IiMxQjFCMUJGRiIsImNvbG9yX21haW5fbGlnaHQiOiIjNDQ0NDQ0RkYiLCJjb2xvcl9wcmltYXJ5IjoiI0YyOEM0NkZGIiwiY29sb3JfdGV4dCI6IiNGRkZGRkZGRiIsImRlcGxveV91dWlkIjoiMmE3ZDY3YjEtOWY3My00ZDI4LWIzZjAtOTgxMGM1YWU2Y2YyIiwiZXhwIjoxNzcyOTU0Mzc2LCJpYXQiOjE3NzI5MTExNzYsIndlYnNpdGUiOiJ3d3cucmp6YXZvcmFsLmNvbSJ9.Mw_sw6eGVLSY5ilWnORW3hPpmAFepjcSJy4oyC5G8w8PFAEbJtW1xXn5ECOBSFEmg6CDGYJwULpb7Jg4Sdhc8PKw4TLQg9MPfgfO_eXM2RYQGrYnw9z5WLWbYOoupJNT3vMBgyKXUfUEPp712tTmVcEc0QV7WfhQxJA2w9OKo83o5i1Ffqk0wp8DmDptJF6cOSwa_8PZwpSivnpMc-vJv0kRCsb9a791Bu4k9brzk_IjRqz-MEqYib8z-gU62vzUQ9XshDAqw7PJVK-RHHobvGE6PMynGYb13jBoqcZM0lW_DkZOlCsP3JTjHAuJOi673yf-HlQF4OT-yw1TunSs8EbK67hz0chmnu0TDrwaKqVp5R3KLRK6bmlVRSjc3EJkEMsdcKq0Eynxp9ptb_FrGD3m9utno4fUPPUDtM951ecMWPTX1JEHgUd8vDNaMCV8rALVYtQBMsQytU-C7fEy4_CTEw5jrMzqWWUSJnhemB4SYmAXTfq0Q-OrRIcSvAMzcxhAnc50VcyhLVEDI9XnU_FISzdxR1ot7lMUdVwerhkPXtmWYtxI-_QBX2gzjmLOquORO38c5K2MA_UxueJ096VHzyoP-EZvtv7AJjltSUKzfQ-Zmui5yfHPj9O5z3ALF5f_fr6clVr-v5XG952ZNbgeJXoLAmTExSt_EJKhfwk";

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() == 3 {
        let payload = parts[1];

        let mut padded_payload = payload.to_string();
        if padded_payload.len() % 4 != 0 {
            padded_payload.push_str(&"=".repeat(4 - padded_payload.len() % 4));
        }

        if let Ok(decoded) = URL_SAFE_NO_PAD.decode(payload) {
            if let Ok(json_str) = String::from_utf8(decoded) {
                println!("Decoded: {}", json_str);
                if let Ok(value) = serde_json::from_str::<Value>(&json_str) {
                    if let Some(exp) = value.get("exp").and_then(|v| v.as_u64()) {
                        println!("Extracted EXP: {}", exp);
                    }
                }
            }
        } else {
            println!("URL_SAFE_NO_PAD failed, trying default base64 with padding");
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&padded_payload) {
                if let Ok(json_str) = String::from_utf8(decoded) {
                    println!("Decoded Standard: {}", json_str);
                    if let Ok(value) = serde_json::from_str::<Value>(&json_str) {
                        if let Some(exp) = value.get("exp").and_then(|v| v.as_u64()) {
                            println!("Extracted EXP Standard: {}", exp);
                        }
                    }
                }
            } else {
                println!("Base64 decode failed completely for payload: {}", payload);
            }
        }
    }
}
