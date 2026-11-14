"""Execute the installer's actual Python probe against unavailable runtime scenarios."""
import contextlib
import io
from pathlib import Path
import re
import sys
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def probe():
    source = (ROOT / 'tools/voice-server/install.ps1').read_text(encoding='utf-8-sig')
    body = source.split('function Test-VoiceGpu(', 1)[1].split('function Get-CudaWheelIndex(', 1)[0]
    return re.search(r'\$check = "([^"]+)"', body).group(1)


class GpuProbe(unittest.TestCase):
    def execute(self, backend='cuda', requested='cuda', available=True, kernels=True, audio=True):
        calls = []
        class Tensor:
            def __matmul__(self, other):
                calls.append('matmul')
                return self
            def __getitem__(self, index):
                return self
            def item(self):
                return 4
        def ones(*args, **kwargs):
            calls.append('kernel')
            if not kernels:
                raise RuntimeError('no kernel image for detected GPU')
            return Tensor()
        cuda = types.SimpleNamespace(is_available=lambda: available, get_device_name=lambda index: 'fixture GPU',
                                     synchronize=lambda: calls.append('synchronize'))
        torch = types.SimpleNamespace(__version__='fixture', version=types.SimpleNamespace(hip='rocm' if backend=='rocm' else None), cuda=cuda, ones=ones)
        modules = {'torch': torch, 'torchaudio': types.SimpleNamespace(__version__='fixture') if audio else None}
        with patch.dict(sys.modules, modules), patch.object(sys, 'argv', ['probe', requested]), contextlib.redirect_stdout(io.StringIO()):
            try:
                exec(probe(), {})
            except SystemExit as e:
                return e.code, calls

    def test_supported_cuda_and_rocm_require_kernels_and_synchronization(self):
        for backend in ['cuda', 'rocm']:
            status,calls=self.execute(backend=backend,requested=backend)
            self.assertEqual(status,0)
            self.assertEqual(calls,['kernel','matmul','synchronize'])

    def test_visible_but_incompatible_gpu_fails(self):
        with self.assertRaisesRegex(RuntimeError, 'no kernel image'):
            self.execute(kernels=False)

    def test_missing_torchaudio_fails(self):
        with self.assertRaises(ModuleNotFoundError):
            self.execute(audio=False)

    def test_torchaudio_binary_load_error_fails(self):
        import builtins
        original = builtins.__import__
        def broken_import(name, *args, **kwargs):
            if name == 'torchaudio':
                raise OSError('torchaudio DLL cannot load')
            return original(name, *args, **kwargs)
        with patch.object(builtins, '__import__', broken_import), self.assertRaisesRegex(OSError, 'DLL'):
            self.execute()

    def test_missing_gpu_and_wrong_backend_fail_without_kernels(self):
        for options in [{'available':False},{'backend':'rocm'}]:
            status,calls=self.execute(**options)
            self.assertEqual(status,1)
            self.assertEqual(calls,[])
