/**
 * An ordinary JavaScript module with no directive. The expander leaves it
 * alone and TypeScript checks it from its JSDoc types.
 *
 * @param {number} value
 * @returns {number}
 */
export function double(value) {
  return value * 2;
}
