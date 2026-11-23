/// Authored structural GGUF envelope; never usable as an inference model.
pub fn structural_gguf() -> Vec<u8> {
    let mut body = b"GGUF".to_vec();
    body.extend(3_u32.to_le_bytes());
    body.extend(1_u64.to_le_bytes()); // One tensor-info record.
    body.extend(1_u64.to_le_bytes()); // One metadata key.
    body.extend(4_u64.to_le_bytes());
    body.extend(b"name");
    body.extend(8_u32.to_le_bytes()); // GGUF string.
    body.extend(4_u64.to_le_bytes());
    body.extend(b"test");
    body.extend([0; 24]);
    body
}
