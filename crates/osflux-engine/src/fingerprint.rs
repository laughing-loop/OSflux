/// FNV-1a hash. Unlike the standard library hasher it gives the same value
/// on every run and platform, so codes derived from it stay comparable.
pub(crate) fn stable_fingerprint(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;

    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}
