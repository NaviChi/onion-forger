import * as $protobuf from "protobufjs";
import Long = require("long");
/** Properties of a TelemetryFrame. */
export interface ITelemetryFrame {

    /** TelemetryFrame tsMs */
    tsMs?: (number|Long|null);

    /** TelemetryFrame kind */
    kind?: (number|null);

    /** TelemetryFrame payload */
    payload?: (Uint8Array|null);
}

/** Represents a TelemetryFrame. */
export class TelemetryFrame implements ITelemetryFrame {

    /**
     * Constructs a new TelemetryFrame.
     * @param [properties] Properties to set
     */
    constructor(properties?: ITelemetryFrame);

    /** TelemetryFrame tsMs. */
    public tsMs: (number|Long);

    /** TelemetryFrame kind. */
    public kind: number;

    /** TelemetryFrame payload. */
    public payload: Uint8Array;

    /**
     * Creates a new TelemetryFrame instance using the specified properties.
     * @param [properties] Properties to set
     * @returns TelemetryFrame instance
     */
    public static create(properties?: ITelemetryFrame): TelemetryFrame;

    /**
     * Encodes the specified TelemetryFrame message. Does not implicitly {@link TelemetryFrame.verify|verify} messages.
     * @param message TelemetryFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encode(message: ITelemetryFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Encodes the specified TelemetryFrame message, length delimited. Does not implicitly {@link TelemetryFrame.verify|verify} messages.
     * @param message TelemetryFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encodeDelimited(message: ITelemetryFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Decodes a TelemetryFrame message from the specified reader or buffer.
     * @param reader Reader or buffer to decode from
     * @param [length] Message length if known beforehand
     * @returns TelemetryFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decode(reader: ($protobuf.Reader|Uint8Array), length?: number): TelemetryFrame;

    /**
     * Decodes a TelemetryFrame message from the specified reader or buffer, length delimited.
     * @param reader Reader or buffer to decode from
     * @returns TelemetryFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decodeDelimited(reader: ($protobuf.Reader|Uint8Array)): TelemetryFrame;

    /**
     * Verifies a TelemetryFrame message.
     * @param message Plain object to verify
     * @returns `null` if valid, otherwise the reason why it is not
     */
    public static verify(message: { [k: string]: any }): (string|null);

    /**
     * Creates a TelemetryFrame message from a plain object. Also converts values to their respective internal types.
     * @param object Plain object
     * @returns TelemetryFrame
     */
    public static fromObject(object: { [k: string]: any }): TelemetryFrame;

    /**
     * Creates a plain object from a TelemetryFrame message. Also converts values to other types if specified.
     * @param message TelemetryFrame
     * @param [options] Conversion options
     * @returns Plain object
     */
    public static toObject(message: TelemetryFrame, options?: $protobuf.IConversionOptions): { [k: string]: any };

    /**
     * Converts this TelemetryFrame to JSON.
     * @returns JSON object
     */
    public toJSON(): { [k: string]: any };

    /**
     * Gets the default type url for TelemetryFrame
     * @param [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns The default type url
     */
    public static getTypeUrl(typeUrlPrefix?: string): string;
}

/** Properties of a ResourceMetricsFrame. */
export interface IResourceMetricsFrame {

    /** ResourceMetricsFrame processCpuPercent */
    processCpuPercent?: (number|null);

    /** ResourceMetricsFrame processMemoryBytes */
    processMemoryBytes?: (number|Long|null);

    /** ResourceMetricsFrame systemMemoryUsedBytes */
    systemMemoryUsedBytes?: (number|Long|null);

    /** ResourceMetricsFrame systemMemoryTotalBytes */
    systemMemoryTotalBytes?: (number|Long|null);

    /** ResourceMetricsFrame activeWorkers */
    activeWorkers?: (number|null);

    /** ResourceMetricsFrame workerTarget */
    workerTarget?: (number|null);

    /** ResourceMetricsFrame activeCircuits */
    activeCircuits?: (number|null);

    /** ResourceMetricsFrame peakActiveCircuits */
    peakActiveCircuits?: (number|null);

    /** ResourceMetricsFrame currentNodeHost */
    currentNodeHost?: (string|null);

    /** ResourceMetricsFrame nodeFailovers */
    nodeFailovers?: (number|null);

    /** ResourceMetricsFrame throttleCount */
    throttleCount?: (number|null);

    /** ResourceMetricsFrame timeoutCount */
    timeoutCount?: (number|null);

    /** ResourceMetricsFrame throttleRatePerSec */
    throttleRatePerSec?: (number|null);

    /** ResourceMetricsFrame phantomPoolDepth */
    phantomPoolDepth?: (number|null);

    /** ResourceMetricsFrame subtreeReroutes */
    subtreeReroutes?: (number|null);

    /** ResourceMetricsFrame subtreeQuarantineHits */
    subtreeQuarantineHits?: (number|null);

    /** ResourceMetricsFrame offWinnerChildRequests */
    offWinnerChildRequests?: (number|null);

    /** ResourceMetricsFrame winnerHost */
    winnerHost?: (string|null);

    /** ResourceMetricsFrame slowestCircuit */
    slowestCircuit?: (string|null);

    /** ResourceMetricsFrame lateThrottles */
    lateThrottles?: (number|null);

    /** ResourceMetricsFrame outlierIsolations */
    outlierIsolations?: (number|null);
}

/** Represents a ResourceMetricsFrame. */
export class ResourceMetricsFrame implements IResourceMetricsFrame {

    /**
     * Constructs a new ResourceMetricsFrame.
     * @param [properties] Properties to set
     */
    constructor(properties?: IResourceMetricsFrame);

    /** ResourceMetricsFrame processCpuPercent. */
    public processCpuPercent: number;

    /** ResourceMetricsFrame processMemoryBytes. */
    public processMemoryBytes: (number|Long);

    /** ResourceMetricsFrame systemMemoryUsedBytes. */
    public systemMemoryUsedBytes: (number|Long);

    /** ResourceMetricsFrame systemMemoryTotalBytes. */
    public systemMemoryTotalBytes: (number|Long);

    /** ResourceMetricsFrame activeWorkers. */
    public activeWorkers: number;

    /** ResourceMetricsFrame workerTarget. */
    public workerTarget: number;

    /** ResourceMetricsFrame activeCircuits. */
    public activeCircuits: number;

    /** ResourceMetricsFrame peakActiveCircuits. */
    public peakActiveCircuits: number;

    /** ResourceMetricsFrame currentNodeHost. */
    public currentNodeHost?: (string|null);

    /** ResourceMetricsFrame nodeFailovers. */
    public nodeFailovers: number;

    /** ResourceMetricsFrame throttleCount. */
    public throttleCount: number;

    /** ResourceMetricsFrame timeoutCount. */
    public timeoutCount: number;

    /** ResourceMetricsFrame throttleRatePerSec. */
    public throttleRatePerSec: number;

    /** ResourceMetricsFrame phantomPoolDepth. */
    public phantomPoolDepth: number;

    /** ResourceMetricsFrame subtreeReroutes. */
    public subtreeReroutes: number;

    /** ResourceMetricsFrame subtreeQuarantineHits. */
    public subtreeQuarantineHits: number;

    /** ResourceMetricsFrame offWinnerChildRequests. */
    public offWinnerChildRequests: number;

    /** ResourceMetricsFrame winnerHost. */
    public winnerHost?: (string|null);

    /** ResourceMetricsFrame slowestCircuit. */
    public slowestCircuit?: (string|null);

    /** ResourceMetricsFrame lateThrottles. */
    public lateThrottles: number;

    /** ResourceMetricsFrame outlierIsolations. */
    public outlierIsolations: number;

    /**
     * Creates a new ResourceMetricsFrame instance using the specified properties.
     * @param [properties] Properties to set
     * @returns ResourceMetricsFrame instance
     */
    public static create(properties?: IResourceMetricsFrame): ResourceMetricsFrame;

    /**
     * Encodes the specified ResourceMetricsFrame message. Does not implicitly {@link ResourceMetricsFrame.verify|verify} messages.
     * @param message ResourceMetricsFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encode(message: IResourceMetricsFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Encodes the specified ResourceMetricsFrame message, length delimited. Does not implicitly {@link ResourceMetricsFrame.verify|verify} messages.
     * @param message ResourceMetricsFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encodeDelimited(message: IResourceMetricsFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Decodes a ResourceMetricsFrame message from the specified reader or buffer.
     * @param reader Reader or buffer to decode from
     * @param [length] Message length if known beforehand
     * @returns ResourceMetricsFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decode(reader: ($protobuf.Reader|Uint8Array), length?: number): ResourceMetricsFrame;

    /**
     * Decodes a ResourceMetricsFrame message from the specified reader or buffer, length delimited.
     * @param reader Reader or buffer to decode from
     * @returns ResourceMetricsFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decodeDelimited(reader: ($protobuf.Reader|Uint8Array)): ResourceMetricsFrame;

    /**
     * Verifies a ResourceMetricsFrame message.
     * @param message Plain object to verify
     * @returns `null` if valid, otherwise the reason why it is not
     */
    public static verify(message: { [k: string]: any }): (string|null);

    /**
     * Creates a ResourceMetricsFrame message from a plain object. Also converts values to their respective internal types.
     * @param object Plain object
     * @returns ResourceMetricsFrame
     */
    public static fromObject(object: { [k: string]: any }): ResourceMetricsFrame;

    /**
     * Creates a plain object from a ResourceMetricsFrame message. Also converts values to other types if specified.
     * @param message ResourceMetricsFrame
     * @param [options] Conversion options
     * @returns Plain object
     */
    public static toObject(message: ResourceMetricsFrame, options?: $protobuf.IConversionOptions): { [k: string]: any };

    /**
     * Converts this ResourceMetricsFrame to JSON.
     * @returns JSON object
     */
    public toJSON(): { [k: string]: any };

    /**
     * Gets the default type url for ResourceMetricsFrame
     * @param [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns The default type url
     */
    public static getTypeUrl(typeUrlPrefix?: string): string;
}

/** Properties of a CrawlStatusFrame. */
export interface ICrawlStatusFrame {

    /** CrawlStatusFrame phase */
    phase?: (string|null);

    /** CrawlStatusFrame progressPercent */
    progressPercent?: (number|null);

    /** CrawlStatusFrame visitedNodes */
    visitedNodes?: (number|Long|null);

    /** CrawlStatusFrame processedNodes */
    processedNodes?: (number|Long|null);

    /** CrawlStatusFrame queuedNodes */
    queuedNodes?: (number|Long|null);

    /** CrawlStatusFrame activeWorkers */
    activeWorkers?: (number|null);

    /** CrawlStatusFrame workerTarget */
    workerTarget?: (number|null);

    /** CrawlStatusFrame etaSeconds */
    etaSeconds?: (number|Long|null);

    /** CrawlStatusFrame deltaNewFiles */
    deltaNewFiles?: (number|Long|null);
}

/** Represents a CrawlStatusFrame. */
export class CrawlStatusFrame implements ICrawlStatusFrame {

    /**
     * Constructs a new CrawlStatusFrame.
     * @param [properties] Properties to set
     */
    constructor(properties?: ICrawlStatusFrame);

    /** CrawlStatusFrame phase. */
    public phase: string;

    /** CrawlStatusFrame progressPercent. */
    public progressPercent: number;

    /** CrawlStatusFrame visitedNodes. */
    public visitedNodes: (number|Long);

    /** CrawlStatusFrame processedNodes. */
    public processedNodes: (number|Long);

    /** CrawlStatusFrame queuedNodes. */
    public queuedNodes: (number|Long);

    /** CrawlStatusFrame activeWorkers. */
    public activeWorkers: number;

    /** CrawlStatusFrame workerTarget. */
    public workerTarget: number;

    /** CrawlStatusFrame etaSeconds. */
    public etaSeconds?: (number|Long|null);

    /** CrawlStatusFrame deltaNewFiles. */
    public deltaNewFiles: (number|Long);

    /**
     * Creates a new CrawlStatusFrame instance using the specified properties.
     * @param [properties] Properties to set
     * @returns CrawlStatusFrame instance
     */
    public static create(properties?: ICrawlStatusFrame): CrawlStatusFrame;

    /**
     * Encodes the specified CrawlStatusFrame message. Does not implicitly {@link CrawlStatusFrame.verify|verify} messages.
     * @param message CrawlStatusFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encode(message: ICrawlStatusFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Encodes the specified CrawlStatusFrame message, length delimited. Does not implicitly {@link CrawlStatusFrame.verify|verify} messages.
     * @param message CrawlStatusFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encodeDelimited(message: ICrawlStatusFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Decodes a CrawlStatusFrame message from the specified reader or buffer.
     * @param reader Reader or buffer to decode from
     * @param [length] Message length if known beforehand
     * @returns CrawlStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decode(reader: ($protobuf.Reader|Uint8Array), length?: number): CrawlStatusFrame;

    /**
     * Decodes a CrawlStatusFrame message from the specified reader or buffer, length delimited.
     * @param reader Reader or buffer to decode from
     * @returns CrawlStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decodeDelimited(reader: ($protobuf.Reader|Uint8Array)): CrawlStatusFrame;

    /**
     * Verifies a CrawlStatusFrame message.
     * @param message Plain object to verify
     * @returns `null` if valid, otherwise the reason why it is not
     */
    public static verify(message: { [k: string]: any }): (string|null);

    /**
     * Creates a CrawlStatusFrame message from a plain object. Also converts values to their respective internal types.
     * @param object Plain object
     * @returns CrawlStatusFrame
     */
    public static fromObject(object: { [k: string]: any }): CrawlStatusFrame;

    /**
     * Creates a plain object from a CrawlStatusFrame message. Also converts values to other types if specified.
     * @param message CrawlStatusFrame
     * @param [options] Conversion options
     * @returns Plain object
     */
    public static toObject(message: CrawlStatusFrame, options?: $protobuf.IConversionOptions): { [k: string]: any };

    /**
     * Converts this CrawlStatusFrame to JSON.
     * @returns JSON object
     */
    public toJSON(): { [k: string]: any };

    /**
     * Gets the default type url for CrawlStatusFrame
     * @param [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns The default type url
     */
    public static getTypeUrl(typeUrlPrefix?: string): string;
}

/** Properties of a BatchProgressFrame. */
export interface IBatchProgressFrame {

    /** BatchProgressFrame completed */
    completed?: (number|Long|null);

    /** BatchProgressFrame failed */
    failed?: (number|Long|null);

    /** BatchProgressFrame total */
    total?: (number|Long|null);

    /** BatchProgressFrame currentFile */
    currentFile?: (string|null);

    /** BatchProgressFrame downloadedBytes */
    downloadedBytes?: (number|Long|null);

    /** BatchProgressFrame activeCircuits */
    activeCircuits?: (number|null);
}

/** Represents a BatchProgressFrame. */
export class BatchProgressFrame implements IBatchProgressFrame {

    /**
     * Constructs a new BatchProgressFrame.
     * @param [properties] Properties to set
     */
    constructor(properties?: IBatchProgressFrame);

    /** BatchProgressFrame completed. */
    public completed: (number|Long);

    /** BatchProgressFrame failed. */
    public failed: (number|Long);

    /** BatchProgressFrame total. */
    public total: (number|Long);

    /** BatchProgressFrame currentFile. */
    public currentFile: string;

    /** BatchProgressFrame downloadedBytes. */
    public downloadedBytes: (number|Long);

    /** BatchProgressFrame activeCircuits. */
    public activeCircuits?: (number|null);

    /**
     * Creates a new BatchProgressFrame instance using the specified properties.
     * @param [properties] Properties to set
     * @returns BatchProgressFrame instance
     */
    public static create(properties?: IBatchProgressFrame): BatchProgressFrame;

    /**
     * Encodes the specified BatchProgressFrame message. Does not implicitly {@link BatchProgressFrame.verify|verify} messages.
     * @param message BatchProgressFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encode(message: IBatchProgressFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Encodes the specified BatchProgressFrame message, length delimited. Does not implicitly {@link BatchProgressFrame.verify|verify} messages.
     * @param message BatchProgressFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encodeDelimited(message: IBatchProgressFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Decodes a BatchProgressFrame message from the specified reader or buffer.
     * @param reader Reader or buffer to decode from
     * @param [length] Message length if known beforehand
     * @returns BatchProgressFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decode(reader: ($protobuf.Reader|Uint8Array), length?: number): BatchProgressFrame;

    /**
     * Decodes a BatchProgressFrame message from the specified reader or buffer, length delimited.
     * @param reader Reader or buffer to decode from
     * @returns BatchProgressFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decodeDelimited(reader: ($protobuf.Reader|Uint8Array)): BatchProgressFrame;

    /**
     * Verifies a BatchProgressFrame message.
     * @param message Plain object to verify
     * @returns `null` if valid, otherwise the reason why it is not
     */
    public static verify(message: { [k: string]: any }): (string|null);

    /**
     * Creates a BatchProgressFrame message from a plain object. Also converts values to their respective internal types.
     * @param object Plain object
     * @returns BatchProgressFrame
     */
    public static fromObject(object: { [k: string]: any }): BatchProgressFrame;

    /**
     * Creates a plain object from a BatchProgressFrame message. Also converts values to other types if specified.
     * @param message BatchProgressFrame
     * @param [options] Conversion options
     * @returns Plain object
     */
    public static toObject(message: BatchProgressFrame, options?: $protobuf.IConversionOptions): { [k: string]: any };

    /**
     * Converts this BatchProgressFrame to JSON.
     * @returns JSON object
     */
    public toJSON(): { [k: string]: any };

    /**
     * Gets the default type url for BatchProgressFrame
     * @param [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns The default type url
     */
    public static getTypeUrl(typeUrlPrefix?: string): string;
}

/** Properties of a DownloadStatusFrame. */
export interface IDownloadStatusFrame {

    /** DownloadStatusFrame phase */
    phase?: (string|null);

    /** DownloadStatusFrame message */
    message?: (string|null);

    /** DownloadStatusFrame downloadTimeSecs */
    downloadTimeSecs?: (number|null);

    /** DownloadStatusFrame percent */
    percent?: (number|null);
}

/** Represents a DownloadStatusFrame. */
export class DownloadStatusFrame implements IDownloadStatusFrame {

    /**
     * Constructs a new DownloadStatusFrame.
     * @param [properties] Properties to set
     */
    constructor(properties?: IDownloadStatusFrame);

    /** DownloadStatusFrame phase. */
    public phase: string;

    /** DownloadStatusFrame message. */
    public message: string;

    /** DownloadStatusFrame downloadTimeSecs. */
    public downloadTimeSecs?: (number|null);

    /** DownloadStatusFrame percent. */
    public percent?: (number|null);

    /**
     * Creates a new DownloadStatusFrame instance using the specified properties.
     * @param [properties] Properties to set
     * @returns DownloadStatusFrame instance
     */
    public static create(properties?: IDownloadStatusFrame): DownloadStatusFrame;

    /**
     * Encodes the specified DownloadStatusFrame message. Does not implicitly {@link DownloadStatusFrame.verify|verify} messages.
     * @param message DownloadStatusFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encode(message: IDownloadStatusFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Encodes the specified DownloadStatusFrame message, length delimited. Does not implicitly {@link DownloadStatusFrame.verify|verify} messages.
     * @param message DownloadStatusFrame message or plain object to encode
     * @param [writer] Writer to encode to
     * @returns Writer
     */
    public static encodeDelimited(message: IDownloadStatusFrame, writer?: $protobuf.Writer): $protobuf.Writer;

    /**
     * Decodes a DownloadStatusFrame message from the specified reader or buffer.
     * @param reader Reader or buffer to decode from
     * @param [length] Message length if known beforehand
     * @returns DownloadStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decode(reader: ($protobuf.Reader|Uint8Array), length?: number): DownloadStatusFrame;

    /**
     * Decodes a DownloadStatusFrame message from the specified reader or buffer, length delimited.
     * @param reader Reader or buffer to decode from
     * @returns DownloadStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    public static decodeDelimited(reader: ($protobuf.Reader|Uint8Array)): DownloadStatusFrame;

    /**
     * Verifies a DownloadStatusFrame message.
     * @param message Plain object to verify
     * @returns `null` if valid, otherwise the reason why it is not
     */
    public static verify(message: { [k: string]: any }): (string|null);

    /**
     * Creates a DownloadStatusFrame message from a plain object. Also converts values to their respective internal types.
     * @param object Plain object
     * @returns DownloadStatusFrame
     */
    public static fromObject(object: { [k: string]: any }): DownloadStatusFrame;

    /**
     * Creates a plain object from a DownloadStatusFrame message. Also converts values to other types if specified.
     * @param message DownloadStatusFrame
     * @param [options] Conversion options
     * @returns Plain object
     */
    public static toObject(message: DownloadStatusFrame, options?: $protobuf.IConversionOptions): { [k: string]: any };

    /**
     * Converts this DownloadStatusFrame to JSON.
     * @returns JSON object
     */
    public toJSON(): { [k: string]: any };

    /**
     * Gets the default type url for DownloadStatusFrame
     * @param [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns The default type url
     */
    public static getTypeUrl(typeUrlPrefix?: string): string;
}
