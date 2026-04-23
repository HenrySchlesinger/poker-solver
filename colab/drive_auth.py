"""Mount Google Drive for Colab precompute output persistence.

Colab runtimes are ephemeral — when the VM dies, `/content` is gone.
Precompute output is too expensive to lose, so we persist it to Drive
under a stable prefix the other notebooks can rely on.

Usage (from a notebook cell):

    from colab.drive_auth import mount_drive
    out = mount_drive()
    # out == '/content/drive/MyDrive/poker-solver'

If `google.colab` is not importable (i.e. the notebook is running outside
Colab — e.g. a local `jupyter nbconvert` sanity-check), `mount_drive`
raises `RuntimeError` so callers fail loudly rather than silently writing
to a path that won't persist.
"""


def mount_drive() -> str:
    """Mount Google Drive and return the poker-solver output root.

    Creates the output directory if it doesn't already exist. Returns
    the absolute path as a string so it can be interpolated directly
    into `subprocess.run` calls or `--output` flags.
    """
    try:
        from google.colab import drive
    except ImportError as e:
        raise RuntimeError(
            "drive_auth.mount_drive() must be called from a Colab runtime; "
            "google.colab is not available in this environment"
        ) from e

    drive.mount("/content/drive")

    import os

    root = "/content/drive/MyDrive/poker-solver"
    os.makedirs(root, exist_ok=True)
    return root
