

export function greet(name: string): string {
  if (!(name.length > 0)) { return "Hello, stranger"; }
  return `Hello, ${name}`;
}
