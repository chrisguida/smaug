import logging
import os
from pathlib import Path

import pytest
from pyln.testing.fixtures import (  # noqa: F401
    db_provider,
    directory,
    env,
    executor,
    jsonschemas,
    network_daemons,
    node_cls,
    node_factory,
    teardown_checks,
    test_base_dir,
    test_name,
)
from pyln.testing.utils import BITCOIND_CONFIG, write_config

SMAUG_PLUGIN = Path.cwd().joinpath("../target/debug/smaug")


# copied from https://github.com/elementsproject/lightning/blob/37ad798a02336a82460b865fd4e6a29d8880856c/contrib/pyln-testing/pyln/testing/fixtures.py#L127-L164
# hacked to include blockfilterindex
@pytest.fixture
def bitcoind(directory, teardown_checks):  # noqa: F811
    chaind = network_daemons[env("TEST_NETWORK", "regtest")]
    bitcoind = chaind(bitcoin_dir=directory)

    BITCOIND_REGTEST = {"rpcport": BITCOIND_CONFIG["rpcport"]}

    BITCOIND_CONFIG["blockfilterindex"] = 1
    BITCOIND_REGTEST["blockfilterindex"] = 1

    conf_file = os.path.join(directory, "bitcoin.conf")
    write_config(conf_file, BITCOIND_CONFIG, BITCOIND_REGTEST)

    try:
        bitcoind.start()
    except Exception:
        bitcoind.stop()
        raise

    info = bitcoind.rpc.getnetworkinfo()

    # FIXME: include liquid-regtest in this check after elementsd has been
    # updated
    if info["version"] < 200100 and env("TEST_NETWORK") != "liquid-regtest":
        bitcoind.rpc.stop()
        raise ValueError(
            "bitcoind is too old. At least version 20100 (v0.20.1)"
            " is needed, current version is {}".format(info["version"])
        )
    elif info["version"] < 160000:
        bitcoind.rpc.stop()
        raise ValueError(
            "elementsd is too old. At least version 160000 (v0.16.0)"
            " is needed, current version is {}".format(info["version"])
        )

    info = bitcoind.rpc.getblockchaininfo()
    # Make sure we have some spendable funds
    if info["blocks"] < 101:
        bitcoind.generate_block(101 - info["blocks"])
    elif bitcoind.rpc.getwalletinfo()["balance"] < 1:
        logging.debug("Insufficient balance, generating 1 block")
        bitcoind.generate_block(1)

    yield bitcoind

    try:
        bitcoind.stop()
    except Exception:
        bitcoind.proc.kill()
    bitcoind.proc.wait()


@pytest.fixture
def ln_node(node_factory):  # noqa: F811
    yield node_factory.get_nodes(
        1,
        opts={
            "allow_broken_log": True,
            "plugin": SMAUG_PLUGIN,
            "smaug_brpc_user": BITCOIND_CONFIG["rpcuser"],
            "smaug_brpc_pass": BITCOIND_CONFIG["rpcpassword"],
            "smaug_brpc_port": BITCOIND_CONFIG["rpcport"],
        },
    )[0]


# SMAUG_PLUGIN = Path("~/.cargo/bin/smaug").expanduser()


# def write_toml_config(filename, opts):
#     with open(filename, "w") as f:
#         for k, v in opts.items():
#             if isinstance(v, str):
#                 f.write('{} = "{}"\n'.format(k, v))
#             else:
#                 f.write("{} = {}\n".format(k, v))

# @pytest.hookimpl(tryfirst=True, hookwrapper=True)
# def pytest_runtest_makereport(item, call):
#     # execute all other hooks to obtain the report object
#     outcome = yield
#     rep = outcome.get_result()

#     # set a report attribute for each phase of a call, which can
#     # be "setup", "call", "teardown"

#     setattr(item, "rep_" + rep.when, rep)


# def pytest_configure(config):
#     config.addinivalue_line("markers", "developer: only run when developer is flagged on")


# def pytest_runtest_setup(item):
#     for mark in item.iter_markers(name="developer"):
#         pass


# @pytest.fixture(scope="function", autouse=True)
# def log_name(request):
#     # Here logging is used, you can use whatever you want to use for logs
#     logging.info("Starting '{}'".format(request.node.name))
