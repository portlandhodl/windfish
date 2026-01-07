/// A Bitcoin Core mempool.dat editor
//
use bitcoin::{
    self, Transaction, Txid, VarInt,
    consensus::{Decodable, Encodable, ReadExt, WriteExt},
};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufReader, Write},
    path::Path,
};

pub const MEMPOOL_DUMP_VERSION_NO_XOR_KEY: u64 = 1;
pub const MEMPOOL_DUMP_VERSION: u64 = 2;

pub type MempoolResult<T> = Result<T, MempoolSerdeError>;

#[derive(Debug)]
pub struct Txn {
    pub tx: bitcoin::Transaction,
    pub time: i64,
    pub fee_delta: i64,
}

#[derive(Debug)]
pub struct MempoolSerde {
    pub version: u64,
    pub txs: Vec<Txn>,
    pub map_deltas: HashMap<Txid, i64>,
    pub unbroadcast_txids: HashSet<Txid>,
}

impl MempoolSerde {
    pub fn new(path: &Path) -> MempoolResult<Self> {
        let mut f = BufReader::new(File::open(path).map_err(MempoolSerdeError::Io)?);

        // Fetch the version as it determines if we have XOR bytes or not.
        let version = f.read_u64()?;

        let mut txs: Vec<Txn> = vec![];
        let mut map_deltas: HashMap<Txid, i64> = HashMap::new();
        let mut unbroadcast_txids: HashSet<Txid> = HashSet::new();

        match version {
            MEMPOOL_DUMP_VERSION_NO_XOR_KEY => {
                // Bytes 9-16 (Number of TXNs)
                for _ in 0..f.read_u64()? {
                    let tx = Transaction::consensus_decode(&mut f)?;
                    let time = f.read_i64()?;
                    let fee_delta = f.read_i64()?;

                    txs.push(Txn {
                        tx,
                        time,
                        fee_delta,
                    });
                }

                // List of fee deltas
                for _ in 0..VarInt::consensus_decode(&mut f)?.0 {
                    let txid = Txid::consensus_decode(&mut f)?;
                    let delta = f.read_i64()?;
                    map_deltas.insert(txid, delta);
                }

                // List of unbroadcast TXIDs
                for _ in 0..VarInt::consensus_decode(&mut f)?.0 {
                    let txid = Txid::consensus_decode(&mut f)?;
                    unbroadcast_txids.insert(txid);
                }
            }
            _ => unimplemented!("Currently V2 (XOR'd) mempool backups are not decodable."),
        }

        Ok(MempoolSerde {
            version,
            txs,
            map_deltas,
            unbroadcast_txids,
        })
    }

    pub fn to_bytes(&self) -> MempoolResult<Vec<u8>> {
        let mut buf = Vec::new();

        buf.emit_u64(self.version)?;
        buf.emit_u64(self.txs.len() as u64)?;

        for txn in &self.txs {
            txn.tx.consensus_encode(&mut buf)?;
            buf.emit_i64(txn.time)?;
            buf.emit_i64(txn.fee_delta)?;
        }

        VarInt(self.map_deltas.len() as u64).consensus_encode(&mut buf)?;
        for (txid, delta) in &self.map_deltas {
            txid.consensus_encode(&mut buf)?;
            buf.emit_i64(*delta)?;
        }

        VarInt(self.unbroadcast_txids.len() as u64).consensus_encode(&mut buf)?;
        for txid in &self.unbroadcast_txids {
            txid.consensus_encode(&mut buf)?;
        }

        Ok(buf)
    }

    pub fn write_to_file(&self, path: &Path) -> MempoolResult<()> {
        let bytes = self.to_bytes()?;
        let mut file = File::create(path).map_err(MempoolSerdeError::Io)?;
        file.write_all(&bytes).map_err(MempoolSerdeError::Io)?;
        Ok(())
    }
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MempoolSerdeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Decode error: {0}")]
    Decode(#[from] bitcoin::consensus::encode::Error),

    #[error("Bitcoin IO error: {0}")]
    BitcoinIo(#[from] bitcoin::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_v1_vector() {
        let mempool = MempoolSerde::new(Path::new("./test/mempool_t4_v1_001.dat")).unwrap();
        assert_eq!(mempool.version, 1);
        assert_ne!(mempool.version, 2);
        println!("{:?}", mempool);
    }

    #[test]
    fn roundtrip_serialization() {
        use bitcoin::hashes::{Hash, sha256};

        let original_bytes = std::fs::read("./test/mempool_t4_v1_001.dat").unwrap();
        let mempool = MempoolSerde::new(Path::new("./test/mempool_t4_v1_001.dat")).unwrap();
        let serialized_bytes = mempool.to_bytes().unwrap();
        let original_hash = sha256::Hash::hash(&original_bytes);
        let serialized_hash = sha256::Hash::hash(&serialized_bytes);

        assert_eq!(original_hash, serialized_hash, "SHA256 hashes don't match");
    }
}
