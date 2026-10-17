// The one node API the node entry point uses (no @types/node needed to build the package).
declare module "node:module" {
  export function createRequire(url: string): (id: string) => unknown;
}
