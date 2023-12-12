from pathlib import Path
import subprocess

from pyln.testing.fixtures import *  # noqa: F401,F403
from pyln.testing.utils import BITCOIND_CONFIG, TailableProc

SMAUG_PLUGIN = Path.cwd().joinpath("../target/debug/smaug")
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
