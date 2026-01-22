def test_rpc_add_with_bip389_descriptor(bitcoind, ln_node):
    """
    Test RPC add.
    """

    multipath_descriptor = "wpkh([b36d6b75/84'/1'/0']tpubDCkkbWNApTd7UqKxCL44mSYgMP3Km72kXunMtX9LLvJvvbtmgEncaW4LJFeeVBy5pBpo2aKVqZc9ZbCGnzHKS3zhxBUSfhLNxL2cKnnY5HB/<0;1>/*)"  # noqa: E501
    # expanded_descriptor_0 = "wpkh([dc182901/84'/1'/0']tpubDCkvbN1aD7W4na5MLZR7pwEn4e6XzNFLfKAfDTyvrQ83Y9WeSWL21xYmUSwaqBa3PaojNBztKiTrYWPRApuux5ZXitvc1wsmYuFFThBX12W/0/*)"  # noqa: E501
    # expanded_descriptor_1 = "wpkh([dc182901/84'/1'/0']tpubDCkvbN1aD7W4na5MLZR7pwEn4e6XzNFLfKAfDTyvrQ83Y9WeSWL21xYmUSwaqBa3PaojNBztKiTrYWPRApuux5ZXitvc1wsmYuFFThBX12W/1/*)"  # noqa: E501

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 0

    # Add a wallet to smaug
    wallet = ln_node.rpc.smaug(
        # "add", expanded_descriptor_0, expanded_descriptor_1, "821000", "5000"
        "add",
        multipath_descriptor,
        "",
        "821000",
        "5000",  # TODO: None
    )
    wallet_name = wallet["name"]

    asserted = {
        "message": f"Wallet with deterministic name {wallet_name} "
        "successfully added",
        "name": wallet_name,
    }

    assert len(wallet.keys()) == len(asserted.keys())
    for key, value in wallet.items():
        assert value == asserted[key]

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 1

    ln_node.daemon.wait_for_log(
        f"Wallet with deterministic name {wallet_name} successfully added"
    )


def test_checksum():
    d = "wpkh([dc182901/84'/1'/0']tpubDCkvbN1aD7W4na5MLZR7pwEn4e6XzNFLfKAfDTyvrQ83Y9WeSWL21xYmUSwaqBa3PaojNBztKiTrYWPRApuux5ZXitvc1wsmYuFFThBX12W/<0;1>/*)"  # noqa: E501
    assert (descsum_create(d)) == d + "#x0j07q3h"


# Below copied from https://github.com/bitcoin/bitcoin/blob/5fbcc8f0560cce36abafb8467339276b7c0d62b6/test/functional/test_framework/descriptors.py  # noqa: E501

# !/usr/bin/env python3
# Copyright (c) 2019 Pieter Wuille
# Distributed under the MIT software license, see the accompanying
# file COPYING or http://www.opensource.org/licenses/mit-license.php.
"""Utility functions related to output descriptors"""

import re  # noqa: E402

INPUT_CHARSET = "0123456789()[],'/*abcdefgh@:$%{}IJKLMNOPQRSTUVWXYZ&+-.;<=>?!^_|~ijklmnopqrstuvwxyzABCDEFGH`#\"\\ "  # noqa: E501
CHECKSUM_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
GENERATOR = [
    0xF5DEE51989,
    0xA9FDCA3312,
    0x1BAB10E32D,
    0x3706B1677A,
    0x644D626FFD,
]


def descsum_polymod(symbols):
    """Internal function that computes the descriptor checksum."""
    chk = 1
    for value in symbols:
        top = chk >> 35
        chk = (chk & 0x7FFFFFFFF) << 5 ^ value
        for i in range(5):
            chk ^= GENERATOR[i] if ((top >> i) & 1) else 0
    return chk


def descsum_expand(s):
    """Internal function that does the character to symbol expansion"""  # noqa: E501
    groups = []
    symbols = []
    for c in s:
        if not c in INPUT_CHARSET:  # noqa: E713
            return None
        v = INPUT_CHARSET.find(c)
        symbols.append(v & 31)
        groups.append(v >> 5)
        if len(groups) == 3:
            symbols.append(groups[0] * 9 + groups[1] * 3 + groups[2])
            groups = []
    if len(groups) == 1:
        symbols.append(groups[0])
    elif len(groups) == 2:
        symbols.append(groups[0] * 3 + groups[1])
    return symbols


def descsum_create(s):
    """Add a checksum to a descriptor without"""
    symbols = descsum_expand(s) + [0, 0, 0, 0, 0, 0, 0, 0]
    checksum = descsum_polymod(symbols) ^ 1
    return (
        s
        + "#"
        + "".join(
            CHECKSUM_CHARSET[(checksum >> (5 * (7 - i))) & 31]
            for i in range(8)
        )
    )


def descsum_check(s, require=True):
    """Verify that the checksum is correct in a descriptor"""
    if not "#" in s:  # noqa: E713
        return not require
    if s[-9] != "#":
        return False
    if not all(x in CHECKSUM_CHARSET for x in s[-8:]):
        return False
    symbols = descsum_expand(s[:-9]) + [
        CHECKSUM_CHARSET.find(x) for x in s[-8:]
    ]
    return descsum_polymod(symbols) == 1


def drop_origins(s):
    """Drop the key origins from a descriptor"""
    desc = re.sub(r"\[.+?\]", "", s)
    if "#" in s:
        desc = desc[: desc.index("#")]
    return descsum_create(desc)
