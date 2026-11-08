# Superteem Pin Protocol v0

Draft used from Step 0. All integers are little-endian.

## Audio frame (L2CAP channel, or GATT notification fallback)

| Offset | Field | Size | Notes |
|---:|---|---:|---|
| 0 | `version` | 1 | `0` for this draft |
| 1 | `flags` | 1 | bit 0 muted, bit 1 flash recording, bit 2 low battery, bit 3 backfill |
| 2 | `pin_session_id` | 2 | Changes each time capture starts |
| 4 | `sequence` | 4 | Increments per frame |
| 8 | `sample_counter` | 4 | First sample of the frame; wraps at 2^32 |
| 12 | `codec` | 1 | `0` raw PCM16, `1` Opus, `2` IMA ADPCM |
| 13 | `payload_length` | 2 | Payload bytes |
| 15 | `payload` | n | One 20 ms frame |

IMA ADPCM is the prototype default. Its payload is the starting predictor (i16), the step index (u8), then 4-bit samples, low nibble first. Each frame can be decoded alone, so a lost frame does not affect later ones. A 20 ms frame at 16 kHz has a 163-byte payload, or about 71 kbps with this header before bundling and BLE overhead.

Test vectors are in `test_vectors.txt`. Both firmware and app must pass them.

## Control messages (GATT characteristic)

Byte 0 is the message type. Payload layouts are finalised at the end of Step 1.

| Type | Direction | Message |
|---:|---|---|
| 0x01 | App to pin | `GetCapabilities` |
| 0x02 | App to pin | `StartStream` |
| 0x03 | App to pin | `StopStream` |
| 0x04 | App to pin | `SetMute` |
| 0x05 | App to pin | `Haptic` |
| 0x06 | App to pin | `ClockPing` |
| 0x07 | App to pin | `BackfillRequest` |
| 0x81 | Pin to app | `Capabilities` |
| 0x82 | Pin to app | `Status` |
| 0x83 | Pin to app | `ButtonEvent` |
| 0x86 | Pin to app | `ClockPong` |
| 0x87 | Pin to app | `BackfillData` |
| 0x88 | Pin to app | `BackfillDone` |
| 0xFF | Pin to app | `Error` |
