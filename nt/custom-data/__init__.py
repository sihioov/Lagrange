"""The Lagrange custom-data package (plan Todo 13).

The package directory is `nt/custom-data`; the hyphen means Python's `import`
statement cannot name it directly, so consumers resolve it through
``importlib.import_module("custom-data.session_events")`` - the same
mechanism NautilusTrader's ``resolve_path`` uses for
``data_cls="custom-data.session_events:SessionOpenEvent"``.  The package is
intentionally import-free (relative imports break when pytest walks the
hyphenated package tree).
"""
