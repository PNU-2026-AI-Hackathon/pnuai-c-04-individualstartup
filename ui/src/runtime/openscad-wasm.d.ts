declare module "openscad-wasm" {
  export interface InitOptions {
    noInitialRun?: boolean;
    print?: (text: string) => void;
    printErr?: (text: string) => void;
  }

  export interface OpenSCAD {
    callMain(args: string[]): number;
    FS: {
      readFile(path: string, options: { encoding: "binary" }): Uint8Array;
      writeFile(path: string, data: string | ArrayBufferView): void;
      unlink(path: string): void;
    };
  }

  export interface OpenSCADInstance {
    getInstance(): OpenSCAD;
  }

  export function createOpenSCAD(options?: InitOptions): Promise<OpenSCADInstance>;
}
