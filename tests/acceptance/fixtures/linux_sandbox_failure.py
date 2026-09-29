#!/usr/bin/env python3
"""Restrict a disposable process before exec, proving worker setup fails closed."""
import ctypes
import os
from pathlib import Path
import platform
import resource
import sys

mode, plugin, worker, cwd = sys.argv[1:]
if platform.system() != "Linux":
    raise SystemExit("Linux is required")
if mode == "rlimit":
    resource.setrlimit(resource.RLIMIT_NOFILE, (32, 32))
else:
    # Add a four-instruction filter; never remove the container's own filter.
    # Deny the one required setup syscall and allow other syscalls to reach the
    # existing filters. The executable's worker must refuse to start afterward.
    number = 444 if mode == "landlock" else {"aarch64": 277, "x86_64": 317}[platform.machine()]
    class Instruction(ctypes.Structure):
        _fields_ = [("code", ctypes.c_ushort), ("jt", ctypes.c_ubyte), ("jf", ctypes.c_ubyte), ("k", ctypes.c_uint)]
    class Program(ctypes.Structure):
        _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.POINTER(Instruction))]
    filters = (Instruction * 4)(Instruction(0x20, 0, 0, 0), Instruction(0x15, 0, 1, number),
                                Instruction(0x06, 0, 0, 0x50001), Instruction(0x06, 0, 0, 0x7fff0000))
    program = Program(4, filters)
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(38, 1, 0, 0, 0) or libc.prctl(22, 2, ctypes.byref(program), 0, 0):
        raise OSError(ctypes.get_errno(), "could not install restrictive setup probe")
os.chdir(cwd)
env = {key: value for key, value in os.environ.items() if not key.startswith(("TYSEL_", "OTEL_", "TRIAGE_", "OPENAI_"))}
env.update(TYSEL_WORKER=str(Path(worker).resolve()), OTEL_SDK_DISABLED="true")
os.execve(plugin, [plugin], env)
