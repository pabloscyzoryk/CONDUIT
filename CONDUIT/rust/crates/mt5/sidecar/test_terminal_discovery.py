"""No MetaTrader import, startup, login or order calls."""
import ntpath
import os
import sys
import unittest
from unittest.mock import patch

import terminal_discovery as discovery


def record(pid=7, session=2, path='C:/Vantage/terminal64.exe'):
    return dict(pid=pid, session=session, path=path)


class TerminalSelectionTests(unittest.TestCase):
    def test_one_current_terminal(self):
        self.assertEqual(discovery.select_terminal([record()], 2), r'c:\vantage\terminal64.exe')

    def test_other_rdp_session_with_unreadable_path_does_not_block(self):
        rows = [record(), record(8, 4, None)]
        self.assertEqual(discovery.select_terminal(rows, 2), r'c:\vantage\terminal64.exe')

    def test_only_other_session_does_not_attach(self):
        with self.assertRaises(discovery.TerminalDiscoveryError):
            discovery.select_terminal([record(8, 4)], 2)

    def test_no_terminal_then_terminal_appears(self):
        with self.assertRaisesRegex(discovery.TerminalDiscoveryError, 'ponowione'):
            discovery.select_terminal([], 2)
        self.assertTrue(discovery.select_terminal([record()], 2))

    def test_multiple_brokers_require_explicit_selection(self):
        rows = [record(), record(8, 2, 'D:/PU Prime/terminal64.exe')]
        with self.assertRaises(discovery.TerminalDiscoveryError):
            discovery.select_terminal(rows, 2)
        self.assertEqual(discovery.select_terminal(rows, 2, 'D:/PU Prime/terminal64.exe'), r'd:\pu prime\terminal64.exe')

    def test_same_binary_two_instances_is_ambiguous_even_with_explicit_path(self):
        rows = [record(), record(8)]
        for chosen in (None, 'C:/Vantage/terminal64.exe'):
            with self.subTest(chosen=chosen), self.assertRaises(discovery.TerminalDiscoveryError):
                discovery.select_terminal(rows, 2, chosen)

    def test_current_unreadable_path_is_not_silently_ignored(self):
        for chosen in (None, 'C:/Vantage/terminal64.exe'):
            with self.subTest(chosen=chosen), self.assertRaises(discovery.TerminalDiscoveryError):
                discovery.select_terminal([record(), record(8, 2, None)], 2, chosen)

    def test_unknown_session_is_not_silently_ignored(self):
        with self.assertRaises(discovery.TerminalDiscoveryError):
            discovery.select_terminal([record(), record(8, None, None)], 2)

    def test_configured_terminal_must_be_running_here(self):
        with self.assertRaises(discovery.TerminalDiscoveryError):
            discovery.select_terminal([record()], 2, 'C:/Old/terminal64.exe')

    def test_unicode_and_case_normalization(self):
        path = 'C:/Program Files/Łódź/TERMINAL64.EXE'
        self.assertEqual(discovery.select_terminal([record(path=path)], 2, path.lower()), ntpath.normcase(ntpath.normpath(path)))

    def test_malformed_inventory_fails_closed(self):
        for rows in ([None], [record(pid=0)], [record(), record()], [record(path='terminal64.exe')], [record(path='C:/Other.exe')]):
            with self.subTest(rows=rows), self.assertRaises(discovery.TerminalDiscoveryError):
                discovery.select_terminal(rows, 2)

    def test_bad_current_session_fails_closed(self):
        for session in (None, -1, '2'):
            with self.subTest(session=session), self.assertRaises(discovery.TerminalDiscoveryError):
                discovery.select_terminal([], session)

    def test_native_boundary_is_used_without_subprocess(self):
        with patch.object(discovery, 'WindowsProcesses') as factory, patch('subprocess.Popen', side_effect=AssertionError('No subprocess')):
            native = factory.return_value
            native.session.return_value = 2
            native.terminals.return_value = [record()]
            self.assertEqual(discovery.running_terminal_path(), r'c:\vantage\terminal64.exe')
            native.session.assert_called_once_with(os.getpid())
            native.terminals.assert_called_once_with(2)

    @unittest.skipUnless(os.name == 'nt', 'Windows API read-only smoke')
    def test_native_limited_query_reads_this_python_process(self):
        native = discovery.WindowsProcesses()
        self.assertIsInstance(native.session(os.getpid()), int)
        self.assertEqual(ntpath.normcase(native.image(os.getpid())), ntpath.normcase(sys.executable))

    @unittest.skipUnless(os.name == 'nt', 'Windows API read-only smoke')
    def test_native_process_inventory_never_initializes_terminal(self):
        native = discovery.WindowsProcesses()
        current = native.session(os.getpid())
        with patch('subprocess.Popen', side_effect=AssertionError('No subprocess')):
            records = native.terminals(current)
        self.assertIsInstance(records, list)
        for row in records:
            self.assertGreater(row['pid'], 0)
            if row['session'] != current:
                self.assertIsNone(row['path'])


if __name__ == '__main__':
    unittest.main()
