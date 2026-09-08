export function validateDocument(document) {
  if (!Array.isArray(document.tasks)) throw new Error('tasks must be an array');
  return document.tasks;
}
