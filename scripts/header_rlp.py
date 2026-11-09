"""RLP-encode an execution block header from an eth_getBlockByNumber result.

    python3 -I scripts/header_rlp.py <block.json>     prints the header's RLP as hex

The field order is the header's, through Prague: each fork only appends fields, and a field
the block doesn't carry is left out. The client checks that keccak of the result is the
block's hash, so a wrong encoding fails loudly rather than proving something else.
"""
import json
import sys

# (JSON key, kind): an int is minimal big-endian, bytes are fixed-length.
FIELDS = [
    ("parentHash", "bytes"),
    ("sha3Uncles", "bytes"),
    ("miner", "bytes"),
    ("stateRoot", "bytes"),
    ("transactionsRoot", "bytes"),
    ("receiptsRoot", "bytes"),
    ("logsBloom", "bytes"),
    ("difficulty", "int"),
    ("number", "int"),
    ("gasLimit", "int"),
    ("gasUsed", "int"),
    ("timestamp", "int"),
    ("extraData", "bytes"),
    ("mixHash", "bytes"),
    ("nonce", "bytes"),
    ("baseFeePerGas", "int"),
    ("withdrawalsRoot", "bytes"),
    ("blobGasUsed", "int"),
    ("excessBlobGas", "int"),
    ("parentBeaconBlockRoot", "bytes"),
    ("requestsHash", "bytes"),
]


def length_prefix(n, short, long_):
    if n < 56:
        return bytes([short + n])
    b = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return bytes([long_ + len(b)]) + b


def rlp_bytes(b):
    if len(b) == 1 and b[0] < 0x80:
        return b
    return length_prefix(len(b), 0x80, 0xB7) + b


def rlp_list(items):
    payload = b"".join(items)
    return length_prefix(len(payload), 0xC0, 0xF7) + payload


def encode(block):
    items = []
    for key, kind in FIELDS:
        if key not in block or block[key] is None:
            break
        raw = block[key]
        if kind == "int":
            n = int(raw, 16)
            items.append(rlp_bytes(n.to_bytes((n.bit_length() + 7) // 8, "big")))
        else:
            items.append(rlp_bytes(bytes.fromhex(raw[2:])))
    return rlp_list(items)


if __name__ == "__main__":
    data = json.load(open(sys.argv[1]))
    print(encode(data.get("result", data)).hex())
