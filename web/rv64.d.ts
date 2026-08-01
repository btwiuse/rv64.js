export type ImageSource =
  | Uint8Array
  | ArrayBuffer
  | ArrayBufferView
  | Response
  | { url: string };

export type BootConfig =
  | {
      mode: "linux-direct";
      kernel: ImageSource;
      initrd?: ImageSource;
      disk?: ImageSource;
      cmdline?: string;
    }
  | {
      mode: "firmware";
      firmware: ImageSource | "default";
      kernel?: ImageSource;
      initrd?: ImageSource;
      disk?: ImageSource;
      cmdline?: string;
    }
  | {
      mode: "bare-metal";
      image: ImageSource;
      loadAddress: bigint;
      entry?: bigint;
      privilege?: "machine" | "supervisor";
    };

export interface DownloadProgress {
  image: "wasm" | "firmware" | "kernel" | "initrd" | "disk" | "image";
  loaded: number;
  total?: number;
}

export interface RV64EventMap {
  ready: undefined;
  start: undefined;
  stop: { reason: "requested" | "powered-off" | "error" };
  error: unknown;
  console: Uint8Array;
  networkTransmit: Uint8Array;
  downloadProgress: DownloadProgress;
}

export type RV64EventListeners = {
  [K in keyof RV64EventMap]?: (event: RV64EventMap[K]) => void;
};

export interface RV64Options {
  wasm: ImageSource;
  memoryMB?: number;
  boot: BootConfig;
  events?: RV64EventListeners;
}

export class RV64 {
  static create(options: RV64Options): Promise<RV64>;
  readonly running: boolean;
  readonly instructions: bigint;
  readonly console: { send(data: string | Uint8Array): void };
  on<K extends keyof RV64EventMap>(
    event: K,
    listener: (event: RV64EventMap[K]) => void,
  ): () => void;
  start(): Promise<void>;
  stop(): Promise<void>;
  reset(): Promise<void>;
  destroy(): Promise<void>;
}
