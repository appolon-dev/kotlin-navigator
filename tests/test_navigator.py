import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from navigator import Index, declarations, imports, symbol_at, utf16_to_py, Server


class NavigationTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        root = Path(self.tempdir.name)
        path_domain = root / "domain" / "src" / "StakingRepository.kt"
        path_feature = root / "feature" / "src" / "StakingViewModel.kt"
        path_domain.parent.mkdir(parents=True)
        path_feature.parent.mkdir(parents=True)
        path_domain.write_text('package com.example.domain\nclass StakingRepository {}\n', encoding='utf-8')
        path_feature.write_text('package com.example.feature\nimport com.example.domain.StakingRepository\nclass StakingViewModel(val repo: StakingRepository)\n', encoding='utf-8')
        self.domain_uri = path_domain.as_uri()
        self.feature_uri = path_feature.as_uri()
        self.idx = Index()
        self.idx.set_roots([root])
        self.assertTrue(self.idx.ready.wait(10))

    def tearDown(self):
        self.tempdir.cleanup()

    def test_cross_module_definition(self):
        definitions = self.idx.definition(self.feature_uri, 2, 36)
        self.assertEqual(len(definitions), 1)
        self.assertEqual(definitions[0]['uri'], self.domain_uri)
        self.assertEqual(definitions[0]['range']['start']['line'], 1)

    def test_unsaved_buffer(self):
        self.idx.update(self.domain_uri, 'package com.example.domain\nclass BrandNewName {}\n')
        found = self.idx.workspace_symbols('BrandNewName')
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0]['location']['uri'], self.domain_uri)

    def test_name_alias(self):
        explicit, star = imports('import com.foo.RealName as Alias\nimport com.bar.*')
        self.assertEqual(explicit['Alias'], 'com.foo.RealName')
        self.assertEqual(star, ['com.bar'])

    def test_utf16(self):
        self.assertEqual(utf16_to_py('😀 Foo', 3), 2)
        self.assertEqual(symbol_at('😀 Foo', 0, 4), 'Foo')

    def test_declarations(self):
        symbols = declarations('package com.a\ninternal data class Wallet {}\nfun interface Clicker {}\n', self.domain_uri)
        self.assertEqual([s.name for s in symbols], ['Wallet', 'Clicker'])



class LspProtocolTests(unittest.TestCase):
    def test_stdio_handshake_and_cross_file_navigation(self):
        import json
        import subprocess
        from navigator import read_message
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'feature').mkdir()
            (root / 'domain').mkdir()
            from_file = root / 'feature' / 'ViewModel.kt'
            to_file = root / 'domain' / 'Repository.kt'
            from_file.write_text('package a.feature\nimport a.domain.Repository\nclass VM(val repository: Repository)\n')
            to_file.write_text('package a.domain\nclass Repository\n')
            process = subprocess.Popen(
                [sys.executable, '-u', str(Path(__file__).resolve().parents[1] / 'navigator.py')],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            try:
                def send(msg):
                    body = json.dumps(msg).encode('utf-8')
                    process.stdin.write(f'Content-Length: {len(body)}\r\n\r\n'.encode('ascii') + body)
                    process.stdin.flush()
                send({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'rootUri': root.as_uri()}})
                reply = read_message(process.stdout)
                self.assertEqual(reply['result']['serverInfo']['name'], 'kotlin-navigator')
                send({'jsonrpc': '2.0', 'method': 'initialized', 'params': {}})
                send({'jsonrpc': '2.0', 'id': 2, 'method': 'textDocument/definition', 'params': {
                    'textDocument': {'uri': from_file.as_uri()}, 'position': {'line': 2, 'character': 28}}})
                reply = read_message(process.stdout)
                self.assertEqual(reply['result'][0]['uri'], to_file.as_uri())
                send({'jsonrpc': '2.0', 'id': 3, 'method': 'shutdown', 'params': {}})
                self.assertIsNone(read_message(process.stdout)['result'])
                send({'jsonrpc': '2.0', 'method': 'exit', 'params': {}})
                process.stdin.close()
                self.assertEqual(process.wait(timeout=5), 0)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                process.stdout.close()
                process.stderr.close()

if __name__ == '__main__':
    unittest.main()
