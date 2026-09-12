"""Read-only MT5 process discovery in the caller's Windows logon session.

Uses limited-query Win32 handles. No PowerShell, WMI, registry fallback,
process launch, terminal initialization or account credentials are involved.
"""
import ctypes
import ntpath
import os
from ctypes import wintypes


class TerminalDiscoveryError(RuntimeError):
    pass


def _path(value):
    if not isinstance(value, str) or not value.strip():
        return None
    value = ntpath.normcase(ntpath.normpath(value.strip()))
    if not ntpath.isabs(value) or ntpath.basename(value) != 'terminal64.exe':
        return None
    return value


def select_terminal(records, current_session, configured=None):
    """Select one proven process; never collapse two PIDs into one filepath."""
    if not isinstance(current_session, int) or current_session < 0:
        raise TerminalDiscoveryError('nie można potwierdzić bieżącej sesji Windows')
    candidates = []
    seen = set()
    for record in records:
        if not isinstance(record, dict):
            raise TerminalDiscoveryError('nieczytelna lista procesów MT5')
        session = record.get('session')
        if session is None:
            raise TerminalDiscoveryError('nie można potwierdzić sesji procesu MT5; wybór terminala wstrzymany')
        if session != current_session:
            continue
        pid = record.get('pid')
        if not isinstance(pid, int) or pid <= 0 or pid in seen:
            raise TerminalDiscoveryError('nieczytelna lista procesów MT5')
        seen.add(pid)
        path = _path(record.get('path'))
        if path is None:
            raise TerminalDiscoveryError('nie można odczytać ścieżki terminala w tej sesji Windows; sprawdź uprawnienia MT5 i bota')
        candidates.append(path)
    if configured:
        wanted = _path(configured)
        if wanted is None:
            raise TerminalDiscoveryError('ścieżka MT5 musi wskazywać pełny plik terminal64.exe')
        candidates = [path for path in candidates if path == wanted]
        if not candidates:
            raise TerminalDiscoveryError('wybrany terminal nie jest uruchomiony w tej sesji Windows; połączenie zostanie ponowione')
    elif not candidates:
        raise TerminalDiscoveryError('MT5 nie jest uruchomiony w tej sesji Windows; połączenie zostanie ponowione')
    if len(candidates) != 1:
        raise TerminalDiscoveryError('działa więcej niż jeden terminal MT5; wskaż jednoznaczną ścieżkę albo zamknij dodatkową instancję')
    return candidates[0]


class _ProcessEntry(ctypes.Structure):
    _fields_ = [('dwSize', wintypes.DWORD), ('cntUsage', wintypes.DWORD),
                ('th32ProcessID', wintypes.DWORD), ('th32DefaultHeapID', ctypes.c_size_t),
                ('th32ModuleID', wintypes.DWORD), ('cntThreads', wintypes.DWORD),
                ('th32ParentProcessID', wintypes.DWORD), ('pcPriClassBase', wintypes.LONG),
                ('dwFlags', wintypes.DWORD), ('szExeFile', wintypes.WCHAR * 260)]


class WindowsProcesses:
    """Small native boundary, separated from deterministic selection tests."""
    def __init__(self):
        if os.name != 'nt':
            raise TerminalDiscoveryError('follow-terminal wymaga Windows')
        self.api = ctypes.WinDLL('kernel32', use_last_error=True)
        signatures = {
            'CreateToolhelp32Snapshot': ([wintypes.DWORD, wintypes.DWORD], wintypes.HANDLE),
            'Process32FirstW': ([wintypes.HANDLE, ctypes.POINTER(_ProcessEntry)], wintypes.BOOL),
            'Process32NextW': ([wintypes.HANDLE, ctypes.POINTER(_ProcessEntry)], wintypes.BOOL),
            'OpenProcess': ([wintypes.DWORD, wintypes.BOOL, wintypes.DWORD], wintypes.HANDLE),
            'QueryFullProcessImageNameW': ([wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR,
                                           ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            'ProcessIdToSessionId': ([wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            'CloseHandle': ([wintypes.HANDLE], wintypes.BOOL),
        }
        for name, (args, result) in signatures.items():
            function = getattr(self.api, name)
            function.argtypes, function.restype = args, result

    def session(self, pid):
        result = wintypes.DWORD()
        if not self.api.ProcessIdToSessionId(pid, ctypes.byref(result)):
            return None
        return int(result.value)

    def image(self, pid):
        # PROCESS_QUERY_LIMITED_INFORMATION avoids the stronger permissions
        # needed by Get-Process.MainModule/Path in some Windows environments.
        handle = self.api.OpenProcess(0x1000, False, pid)
        if not handle:
            return None
        try:
            length = wintypes.DWORD(32768)
            buffer = ctypes.create_unicode_buffer(length.value)
            if self.api.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(length)):
                return buffer.value
            return None
        finally:
            self.api.CloseHandle(handle)

    def terminals(self, current_session):
        snapshot = self.api.CreateToolhelp32Snapshot(0x00000002, 0)
        if snapshot in (None, wintypes.HANDLE(-1).value):
            raise TerminalDiscoveryError('nie można odczytać listy procesów Windows')
        records = []
        try:
            entry = _ProcessEntry()
            entry.dwSize = ctypes.sizeof(entry)
            exists = self.api.Process32FirstW(snapshot, ctypes.byref(entry))
            while exists:
                if entry.szExeFile.casefold() == 'terminal64.exe':
                    pid = int(entry.th32ProcessID)
                    session = self.session(pid)
                    # Do not inspect paths of other RDP/logon sessions. An
                    # unreadable path there must not hide the user's terminal.
                    path = self.image(pid) if session == current_session else None
                    records.append({'pid': pid, 'session': session, 'path': path})
                exists = self.api.Process32NextW(snapshot, ctypes.byref(entry))
            if ctypes.get_last_error() != 18:  # ERROR_NO_MORE_FILES
                raise TerminalDiscoveryError('lista procesów Windows jest niepełna')
        finally:
            self.api.CloseHandle(snapshot)
        return records


def running_terminal_path(configured=None):
    native = WindowsProcesses()
    session = native.session(os.getpid())
    if session is None:
        raise TerminalDiscoveryError('nie można potwierdzić bieżącej sesji Windows')
    return select_terminal(native.terminals(session), session, configured)
