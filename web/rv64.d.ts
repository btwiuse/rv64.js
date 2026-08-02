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

export type NetworkConfig =
  | { mode: "none" }
  | {
      /** HTTP request egress through the host's fetch() implementation. */
      mode: "fetch";
      /** Rewrite plaintext upstream URLs to HTTPS; defaults to true in browsers. */
      upgradeHttps?: boolean;
      /** Optional request-level WebSocket fallback for requests blocked by CORS. */
      relayURL?: string;
      mac?: Uint8Array;
    }
  | {
      /** Raw Ethernet frames over a websockproxy-compatible WebSocket. */
      mode: "wsproxy";
      /** Layer-2 websockproxy-compatible relay URL. */
      url: string;
      protocols?: string | string[];
      mac?: Uint8Array;
    }
  | {
      /** TCP/UDP payload transport through a WISP-compatible relay. */
      mode: "wisp";
      url: string;
      protocols?: string | string[];
      mac?: Uint8Array;
    }
  | {
      /** Browser-local Ethernet shared over BroadcastChannel. */
      mode: "inbrowser";
      channel?: string;
      mac?: Uint8Array;
    }
  | { mode: "external"; mac?: Uint8Array };

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
  /** Linux defaults to the built-in fetch backend; bare metal defaults to none. */
  network?: NetworkConfig;
  events?: RV64EventListeners;
}

export class RV64 {
  static create(options: RV64Options): Promise<RV64>;
  readonly running: boolean;
  readonly instructions: bigint;
  readonly console: { send(data: string | Uint8Array): void };
  readonly network: {
    readonly mode: NetworkConfig["mode"];
    readonly proxyURL?: string;
    /** Supply an Ethernet frame when network.mode is "external". */
    receive(frame: Uint8Array | ArrayBuffer): void;
  };
  on<K extends keyof RV64EventMap>(
    event: K,
    listener: (event: RV64EventMap[K]) => void,
  ): () => void;
  start(): Promise<void>;
  stop(): Promise<void>;
  reset(): Promise<void>;
  destroy(): Promise<void>;
}
