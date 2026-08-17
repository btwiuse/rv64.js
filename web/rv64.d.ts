export type ImageSource =
  | Uint8Array
  | ArrayBuffer
  | ArrayBufferView
  | Response
  | { url: string };

export type ExecutionConfig =
  | { mode: "local" }
  | {
      /** Run Wasm, devices, image downloads, and networking in a dedicated Worker. */
      mode: "worker";
      /** Module Worker entry point; defaults to rv64.worker.js beside rv64.js. */
      workerURL?: string | URL;
      /** Frequency of cached running/instruction state updates; defaults to 500ms. */
      statisticsIntervalMs?: number;
    };

export type BootConfig =
  | {
      mode: "linux-direct";
      kernel: ImageSource;
      initrd?: ImageSource;
      disk?: ImageSource;
      cmdline?: string;
      /** Delegate a virtio-9P mount to an asynchronous host handler. Local execution only. */
      p9?: { tag: string; handle(request: Uint8Array): Uint8Array | Promise<Uint8Array> };
      /** Add a secondary virtio console, used by integrations such as WANIX host export. */
      virtioConsole?: boolean;
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
  /** Bytes from the optional secondary virtio-console host-export channel. */
  export: Uint8Array;
  networkTransmit: Uint8Array;
  networkTraffic:
    | { type: "request"; bytes: number; method: string; url: string }
    | { type: "response"; status: number; url?: string }
    | { type: "download"; bytes: number }
    | { type: "end"; url?: string }
    | { type: "error"; message: string; url?: string };
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
  /** Execution stays on the calling thread unless Worker mode is explicitly selected. */
  execution?: ExecutionConfig;
  events?: RV64EventListeners;
}

export interface JitConfiguration {
  policy?: "page" | "adaptive";
  pageThreshold?: number;
  privilegedPageThresholdMultiplier?: number;
  pageQuantum?: number;
  regionLeaderCap?: number;
  regionPageCap?: number;
  regionExtensionPageCap?: number;
  measuredRegionMinStay?: number;
  pageInflightLimit?: number;
  multiPageEntryCap?: number;
  multiPageControlPermille?: number;
  regionTlbCacheMinAccesses?: number;
  profileSampleShift?: number;
  directDispatch?: boolean;
  lazyState?: boolean;
  pageRebuild?: boolean;
  controlEntries?: boolean;
  privilegedControlEntries?: boolean;
  stablePageChain?: boolean;
  controlProfile?: boolean;
  crossPageCalls?: boolean;
  measuredRegions?: boolean;
  demoteRegions?: boolean;
  cfgBlocks?: boolean;
  structuredCfg?: boolean;
  tlbFill?: boolean;
  tlbHash?: boolean;
  privilegeTlbRetention?: boolean;
  regionTlbCache?: boolean;
  regionChain?: boolean;
  regionTailChain?: boolean;
  profile?: boolean;
}

export type JitStringCounters = Readonly<Record<string, string>>;

export interface JitGeneratedStats {
  retired: string;
  dispatches: string;
  userEntries: string;
  systemEntries: string;
  zeroRetireDispatches: string;
  zeroRetireSuppressions: string;
  dispatchEmptyMisses: string;
  dispatchTagCollisions: string;
  dispatchReverifications: string;
  zeroRetireTracked: string;
  zeroRetireUntracked: string;
  zeroRetireProfileResets: string;
  zeroRetireProfiles: string;
  zeroRetireSuppressedEntries: string;
  tlbFills: string;
  chainHops: string;
  tlbFillKinds: JitStringCounters;
}

export interface JitStats {
  generated: JitGeneratedStats;
  mmu: JitStringCounters;
  interpreter: JitStringCounters;
  staticT0: Readonly<Record<string, string | number | boolean>>;
  regions: JitStringCounters;
  translation: JitStringCounters;
  pagePolicy: JitStringCounters;
  loader: Readonly<Record<string, number>>;
  p9: Readonly<Record<string, number>>;
  instructions: string;
  accountedInstructions: string;
  generatedCoverage: number;
  generatedInstructionsPerDispatch: number;
}

export interface JitDispatchProfile {
  pc: string;
  calls: number;
  retired: number;
  instructionsPerCall: number;
}

export interface JitEdgeProfile {
  source: string;
  target: string;
  transitions: number;
  retired: number;
  instructionsPerTransition: number;
}

export interface JitFallbackProfile {
  key: string;
  stretches: number;
  interpretedInstructions: number;
}

export interface JitProfile {
  generatedBlockCalls: string;
  generatedBlockInstructions: string;
  generatedRegionCalls: string;
  generatedRegionInstructions: string;
  dispatch: JitDispatchProfile[];
  edges: JitEdgeProfile[];
  fallback: JitFallbackProfile[];
}

export class RV64 {
  static create(options: RV64Options): Promise<RV64>;
  readonly running: boolean;
  /** Exact in local mode; periodically sampled in Worker mode. */
  readonly instructions: bigint;
  readonly console: { send(data: string | Uint8Array): void };
  readonly export: { send(data: string | Uint8Array): void };
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
  /** Enable or disable generated-code tiering without stopping the machine. */
  setJitEnabled(enabled: boolean): void | Promise<void>;
  /** Apply bounded JIT policy controls; omitted fields retain their values. */
  configureJit(options: JitConfiguration): void | Promise<void>;
  /** Return exact cumulative counters. No background polling is installed. */
  jitStats(): JitStats | Promise<JitStats>;
  /** Return opt-in profiling data configured with `profile: true`. */
  jitProfile(limit?: number): JitProfile | Promise<JitProfile>;
  start(): Promise<void>;
  stop(): Promise<void>;
  reset(): Promise<void>;
  destroy(): Promise<void>;
}
