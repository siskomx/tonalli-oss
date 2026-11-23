"""Recognize only unpublished output from an exact adb exec-out cat request."""


def cat_pending(stdout, requested_path):
    # exec-out may report remote cat failure on stdout with a zero exit code.
    return stdout.strip() in (b"", f"cat: {requested_path}: No such file or directory".encode())
