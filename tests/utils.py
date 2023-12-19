from pyln.testing.utils import only_one


def sats_to_btc(sats):
    return sats / 10**8


def btc_to_sats(btc):
    return btc * 10**8


def get_bitcoind_wallet_bal_sats(bitcoind):
    wi_res = bitcoind.rpc.getwalletinfo()
    return int(btc_to_sats(wi_res["balance"] + wi_res["immature_balance"]))


def get_bkpr_smaug_balance(name, bkpr_balances):
    return only_one(
        only_one(
            list(filter(lambda x: x["account"] == "smaug:%s" % name, bkpr_balances))
        )["balances"]
    )


def get_cln_balance(balances):
    return only_one(
        only_one(list(filter(lambda x: x["account"] == "wallet", balances)))["balances"]
    )


def get_descriptor(wpkh_descriptors, internal):
    return only_one(
        list(filter(lambda x: x["internal"] is internal, wpkh_descriptors))
    )["desc"]


def get_only_one_descriptor(bitcoind, script_type, internal):
    all_descriptors = bitcoind.rpc.listdescriptors()["descriptors"]
    descriptors = list(
        filter(lambda x: x["desc"].startswith(script_type), all_descriptors)
    )
    return get_descriptor(descriptors, internal)


def switch_wallet(bitcoind, wallet_name):
    current_wallets = bitcoind.rpc.listwallets()
    if wallet_name not in current_wallets:
        bitcoind.rpc.loadwallet(wallet_name)
    for w in current_wallets:
        if w != wallet_name:
            bitcoind.rpc.unloadwallet(w)

def send_from_wallet(bitcoind, wallet_name, address, amount):
    current_wallet_name = only_one(bitcoind.rpc.listwallets())
    if current_wallet_name != wallet_name:
        switch_wallet(bitcoind, wallet_name)
    bitcoind.rpc.sendtoaddress(address, amount)
    if current_wallet_name != wallet_name:
        switch_wallet(bitcoind, current_wallet_name)

def generate_to_mining_wallet(bitcoind, mining_wallet_name, transacting_wallet_name, num_blocks=1):
    switch_wallet(bitcoind, mining_wallet_name)
    bitcoind.generate_block(num_blocks)
    switch_wallet(bitcoind, transacting_wallet_name)
