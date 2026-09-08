"""Hash the complete task evaluator, separately from participant fixture files."""
import hashlib


def evaluator_files(task):
    return {str(file.relative_to(task)): hashlib.sha256(file.read_bytes()).hexdigest()
            for file in sorted(task.rglob('*.mjs'))
            if 'fixture' not in file.relative_to(task).parts}
