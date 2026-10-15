

const trim = (value: string) => value.trim();
const shout = (value: string) => value.toUpperCase();
export const result = shout(trim("  hello  "));
