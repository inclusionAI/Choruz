"""Reject damaged, misplaced or unsafe CI release archives before publication."""

import json
from pathlib import Path, PurePosixPath
import sys
import tarfile
import tempfile

from release import verify


def verify_archive(archive, revision):
    with tarfile.open(archive) as bundle, tempfile.TemporaryDirectory(prefix="choruz-verify-") as temp:
        members = bundle.getmembers()
        roots = set()
        for member in members:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or not path.parts:
                raise ValueError("archive member escapes its release directory")
            roots.add(path.parts[0])
        if len(roots) != 1:
            raise ValueError("archive must contain exactly one release directory")
        # Python's data filter rejects escaping links and device files as well.
        bundle.extractall(temp, filter="data")
        manifest = verify(Path(temp) / roots.pop())
        if manifest["revision"] != revision:
            raise ValueError("artifact revision does not match the verified CI run")
        return manifest


if __name__ == "__main__":
    result = verify_archive(sys.argv[1], sys.argv[2])
    print(json.dumps({key: value for key, value in result.items() if key != "files"}))
