/// Magic headers identifying ESF files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsfType {
    /// Four-byte header `0xABCD`. Earlier ESF format.
    ABCD = 0xABCD,
    /// Four-byte header `0xABCE`. Later ESF format.
    ABCE = 0xABCE,
}

impl TryFrom<u32> for EsfType {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0xABCD => Ok(EsfType::ABCD),
            0xABCE => Ok(EsfType::ABCE),
            _ => Err(format!("Unsupported ESF magic header type: 0x{:04X}", value)),
        }
    }
}

/// Type tags for leaf values and structural nodes in an ESF file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsfValueType {
    Ascii = 0x0f,
    Binary41 = 0x41,
    Binary42 = 0x42,
    Binary43 = 0x43,
    Binary44 = 0x44,
    Binary45 = 0x45,
    Binary46 = 0x46,
    Binary47 = 0x47,
    Binary48 = 0x48,
    Binary49 = 0x49,
    Binary4A = 0x4a,
    Binary4B = 0x4b,
    Binary4C = 0x4c,
    Binary4D = 0x4d,
    Boolean = 1,
    Byte = 6,
    Float = 0x0a,
    FloatPoint = 0x0c,
    FloatPoint3D = 0x0d,
    Int = 4,
    PolyNode = 0x81,
    Short = 0,
    SingleNode = 0x80,
    UInt = 8,
    UInt64 = 9,
    UInt16 = 7,
    UShort = 0x10,
    UTF16 = 0x0e,
    Unknown109 = 109,
    OptimizedBlock140 = 140,
}

impl TryFrom<u8> for EsfValueType {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x0f => Ok(EsfValueType::Ascii),
            0x41 => Ok(EsfValueType::Binary41),
            0x42 => Ok(EsfValueType::Binary42),
            0x43 => Ok(EsfValueType::Binary43),
            0x44 => Ok(EsfValueType::Binary44),
            0x45 => Ok(EsfValueType::Binary45),
            0x46 => Ok(EsfValueType::Binary46),
            0x47 => Ok(EsfValueType::Binary47),
            0x48 => Ok(EsfValueType::Binary48),
            0x49 => Ok(EsfValueType::Binary49),
            0x4a => Ok(EsfValueType::Binary4A),
            0x4b => Ok(EsfValueType::Binary4B),
            0x4c => Ok(EsfValueType::Binary4C),
            0x4d => Ok(EsfValueType::Binary4D),
            1 => Ok(EsfValueType::Boolean),
            6 => Ok(EsfValueType::Byte),
            0x0a => Ok(EsfValueType::Float),
            0x0c => Ok(EsfValueType::FloatPoint),
            0x0d => Ok(EsfValueType::FloatPoint3D),
            4 => Ok(EsfValueType::Int),
            0x81 => Ok(EsfValueType::PolyNode),
            0 => Ok(EsfValueType::Short),
            0x80 => Ok(EsfValueType::SingleNode),
            8 => Ok(EsfValueType::UInt),
            9 => Ok(EsfValueType::UInt64),
            7 => Ok(EsfValueType::UInt16),
            0x10 => Ok(EsfValueType::UShort),
            0x0e => Ok(EsfValueType::UTF16),
            109 => Ok(EsfValueType::Unknown109),
            140 => Ok(EsfValueType::OptimizedBlock140),
            _ => Err(format!("Unknown EsfValueType byte: 0x{:02X}", value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_esf_type_try_from() {
        assert_eq!(EsfType::try_from(0xABCD), Ok(EsfType::ABCD));
        assert_eq!(EsfType::try_from(0xABCE), Ok(EsfType::ABCE));
        assert!(EsfType::try_from(0x1234).is_err());
    }

    #[test]
    fn test_esf_value_type_try_from() {
        assert_eq!(EsfValueType::try_from(0x0f), Ok(EsfValueType::Ascii));
        assert_eq!(EsfValueType::try_from(1), Ok(EsfValueType::Boolean));
        assert!(EsfValueType::try_from(0xFF).is_err());
    }
}
