/*eslint-disable block-scoped-var, id-length, no-control-regex, no-magic-numbers, no-prototype-builtins, no-redeclare, no-shadow, no-var, sort-vars*/
import * as $protobuf from "protobufjs/minimal";

// Common aliases
const $Reader = $protobuf.Reader, $Writer = $protobuf.Writer, $util = $protobuf.util;

// Exported root namespace
const $root = $protobuf.roots["default"] || ($protobuf.roots["default"] = {});

export const TelemetryFrame = $root.TelemetryFrame = (() => {

    /**
     * Properties of a TelemetryFrame.
     * @exports ITelemetryFrame
     * @interface ITelemetryFrame
     * @property {number|Long|null} [tsMs] TelemetryFrame tsMs
     * @property {number|null} [kind] TelemetryFrame kind
     * @property {Uint8Array|null} [payload] TelemetryFrame payload
     */

    /**
     * Constructs a new TelemetryFrame.
     * @exports TelemetryFrame
     * @classdesc Represents a TelemetryFrame.
     * @implements ITelemetryFrame
     * @constructor
     * @param {ITelemetryFrame=} [properties] Properties to set
     */
    function TelemetryFrame(properties) {
        if (properties)
            for (let keys = Object.keys(properties), i = 0; i < keys.length; ++i)
                if (properties[keys[i]] != null)
                    this[keys[i]] = properties[keys[i]];
    }

    /**
     * TelemetryFrame tsMs.
     * @member {number|Long} tsMs
     * @memberof TelemetryFrame
     * @instance
     */
    TelemetryFrame.prototype.tsMs = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * TelemetryFrame kind.
     * @member {number} kind
     * @memberof TelemetryFrame
     * @instance
     */
    TelemetryFrame.prototype.kind = 0;

    /**
     * TelemetryFrame payload.
     * @member {Uint8Array} payload
     * @memberof TelemetryFrame
     * @instance
     */
    TelemetryFrame.prototype.payload = $util.newBuffer([]);

    /**
     * Creates a new TelemetryFrame instance using the specified properties.
     * @function create
     * @memberof TelemetryFrame
     * @static
     * @param {ITelemetryFrame=} [properties] Properties to set
     * @returns {TelemetryFrame} TelemetryFrame instance
     */
    TelemetryFrame.create = function create(properties) {
        return new TelemetryFrame(properties);
    };

    /**
     * Encodes the specified TelemetryFrame message. Does not implicitly {@link TelemetryFrame.verify|verify} messages.
     * @function encode
     * @memberof TelemetryFrame
     * @static
     * @param {ITelemetryFrame} message TelemetryFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    TelemetryFrame.encode = function encode(message, writer) {
        if (!writer)
            writer = $Writer.create();
        if (message.tsMs != null && Object.hasOwnProperty.call(message, "tsMs"))
            writer.uint32(/* id 1, wireType 0 =*/8).uint64(message.tsMs);
        if (message.kind != null && Object.hasOwnProperty.call(message, "kind"))
            writer.uint32(/* id 2, wireType 0 =*/16).uint32(message.kind);
        if (message.payload != null && Object.hasOwnProperty.call(message, "payload"))
            writer.uint32(/* id 3, wireType 2 =*/26).bytes(message.payload);
        return writer;
    };

    /**
     * Encodes the specified TelemetryFrame message, length delimited. Does not implicitly {@link TelemetryFrame.verify|verify} messages.
     * @function encodeDelimited
     * @memberof TelemetryFrame
     * @static
     * @param {ITelemetryFrame} message TelemetryFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    TelemetryFrame.encodeDelimited = function encodeDelimited(message, writer) {
        return this.encode(message, writer).ldelim();
    };

    /**
     * Decodes a TelemetryFrame message from the specified reader or buffer.
     * @function decode
     * @memberof TelemetryFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @param {number} [length] Message length if known beforehand
     * @returns {TelemetryFrame} TelemetryFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    TelemetryFrame.decode = function decode(reader, length, error) {
        if (!(reader instanceof $Reader))
            reader = $Reader.create(reader);
        let end = length === undefined ? reader.len : reader.pos + length, message = new $root.TelemetryFrame();
        while (reader.pos < end) {
            let tag = reader.uint32();
            if (tag === error)
                break;
            switch (tag >>> 3) {
            case 1: {
                    message.tsMs = reader.uint64();
                    break;
                }
            case 2: {
                    message.kind = reader.uint32();
                    break;
                }
            case 3: {
                    message.payload = reader.bytes();
                    break;
                }
            default:
                reader.skipType(tag & 7);
                break;
            }
        }
        return message;
    };

    /**
     * Decodes a TelemetryFrame message from the specified reader or buffer, length delimited.
     * @function decodeDelimited
     * @memberof TelemetryFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @returns {TelemetryFrame} TelemetryFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    TelemetryFrame.decodeDelimited = function decodeDelimited(reader) {
        if (!(reader instanceof $Reader))
            reader = new $Reader(reader);
        return this.decode(reader, reader.uint32());
    };

    /**
     * Verifies a TelemetryFrame message.
     * @function verify
     * @memberof TelemetryFrame
     * @static
     * @param {Object.<string,*>} message Plain object to verify
     * @returns {string|null} `null` if valid, otherwise the reason why it is not
     */
    TelemetryFrame.verify = function verify(message) {
        if (typeof message !== "object" || message === null)
            return "object expected";
        if (message.tsMs != null && message.hasOwnProperty("tsMs"))
            if (!$util.isInteger(message.tsMs) && !(message.tsMs && $util.isInteger(message.tsMs.low) && $util.isInteger(message.tsMs.high)))
                return "tsMs: integer|Long expected";
        if (message.kind != null && message.hasOwnProperty("kind"))
            if (!$util.isInteger(message.kind))
                return "kind: integer expected";
        if (message.payload != null && message.hasOwnProperty("payload"))
            if (!(message.payload && typeof message.payload.length === "number" || $util.isString(message.payload)))
                return "payload: buffer expected";
        return null;
    };

    /**
     * Creates a TelemetryFrame message from a plain object. Also converts values to their respective internal types.
     * @function fromObject
     * @memberof TelemetryFrame
     * @static
     * @param {Object.<string,*>} object Plain object
     * @returns {TelemetryFrame} TelemetryFrame
     */
    TelemetryFrame.fromObject = function fromObject(object) {
        if (object instanceof $root.TelemetryFrame)
            return object;
        let message = new $root.TelemetryFrame();
        if (object.tsMs != null)
            if ($util.Long)
                (message.tsMs = $util.Long.fromValue(object.tsMs)).unsigned = true;
            else if (typeof object.tsMs === "string")
                message.tsMs = parseInt(object.tsMs, 10);
            else if (typeof object.tsMs === "number")
                message.tsMs = object.tsMs;
            else if (typeof object.tsMs === "object")
                message.tsMs = new $util.LongBits(object.tsMs.low >>> 0, object.tsMs.high >>> 0).toNumber(true);
        if (object.kind != null)
            message.kind = object.kind >>> 0;
        if (object.payload != null)
            if (typeof object.payload === "string")
                $util.base64.decode(object.payload, message.payload = $util.newBuffer($util.base64.length(object.payload)), 0);
            else if (object.payload.length >= 0)
                message.payload = object.payload;
        return message;
    };

    /**
     * Creates a plain object from a TelemetryFrame message. Also converts values to other types if specified.
     * @function toObject
     * @memberof TelemetryFrame
     * @static
     * @param {TelemetryFrame} message TelemetryFrame
     * @param {$protobuf.IConversionOptions} [options] Conversion options
     * @returns {Object.<string,*>} Plain object
     */
    TelemetryFrame.toObject = function toObject(message, options) {
        if (!options)
            options = {};
        let object = {};
        if (options.defaults) {
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.tsMs = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.tsMs = options.longs === String ? "0" : 0;
            object.kind = 0;
            if (options.bytes === String)
                object.payload = "";
            else {
                object.payload = [];
                if (options.bytes !== Array)
                    object.payload = $util.newBuffer(object.payload);
            }
        }
        if (message.tsMs != null && message.hasOwnProperty("tsMs"))
            if (typeof message.tsMs === "number")
                object.tsMs = options.longs === String ? String(message.tsMs) : message.tsMs;
            else
                object.tsMs = options.longs === String ? $util.Long.prototype.toString.call(message.tsMs) : options.longs === Number ? new $util.LongBits(message.tsMs.low >>> 0, message.tsMs.high >>> 0).toNumber(true) : message.tsMs;
        if (message.kind != null && message.hasOwnProperty("kind"))
            object.kind = message.kind;
        if (message.payload != null && message.hasOwnProperty("payload"))
            object.payload = options.bytes === String ? $util.base64.encode(message.payload, 0, message.payload.length) : options.bytes === Array ? Array.prototype.slice.call(message.payload) : message.payload;
        return object;
    };

    /**
     * Converts this TelemetryFrame to JSON.
     * @function toJSON
     * @memberof TelemetryFrame
     * @instance
     * @returns {Object.<string,*>} JSON object
     */
    TelemetryFrame.prototype.toJSON = function toJSON() {
        return this.constructor.toObject(this, $protobuf.util.toJSONOptions);
    };

    /**
     * Gets the default type url for TelemetryFrame
     * @function getTypeUrl
     * @memberof TelemetryFrame
     * @static
     * @param {string} [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns {string} The default type url
     */
    TelemetryFrame.getTypeUrl = function getTypeUrl(typeUrlPrefix) {
        if (typeUrlPrefix === undefined) {
            typeUrlPrefix = "type.googleapis.com";
        }
        return typeUrlPrefix + "/TelemetryFrame";
    };

    return TelemetryFrame;
})();

export const ResourceMetricsFrame = $root.ResourceMetricsFrame = (() => {

    /**
     * Properties of a ResourceMetricsFrame.
     * @exports IResourceMetricsFrame
     * @interface IResourceMetricsFrame
     * @property {number|null} [processCpuPercent] ResourceMetricsFrame processCpuPercent
     * @property {number|Long|null} [processMemoryBytes] ResourceMetricsFrame processMemoryBytes
     * @property {number|Long|null} [systemMemoryUsedBytes] ResourceMetricsFrame systemMemoryUsedBytes
     * @property {number|Long|null} [systemMemoryTotalBytes] ResourceMetricsFrame systemMemoryTotalBytes
     * @property {number|null} [activeWorkers] ResourceMetricsFrame activeWorkers
     * @property {number|null} [workerTarget] ResourceMetricsFrame workerTarget
     * @property {number|null} [activeCircuits] ResourceMetricsFrame activeCircuits
     * @property {number|null} [peakActiveCircuits] ResourceMetricsFrame peakActiveCircuits
     * @property {string|null} [currentNodeHost] ResourceMetricsFrame currentNodeHost
     * @property {number|null} [nodeFailovers] ResourceMetricsFrame nodeFailovers
     * @property {number|null} [throttleCount] ResourceMetricsFrame throttleCount
     * @property {number|null} [timeoutCount] ResourceMetricsFrame timeoutCount
     * @property {number|null} [throttleRatePerSec] ResourceMetricsFrame throttleRatePerSec
     * @property {number|null} [phantomPoolDepth] ResourceMetricsFrame phantomPoolDepth
     * @property {number|null} [subtreeReroutes] ResourceMetricsFrame subtreeReroutes
     * @property {number|null} [subtreeQuarantineHits] ResourceMetricsFrame subtreeQuarantineHits
     * @property {number|null} [offWinnerChildRequests] ResourceMetricsFrame offWinnerChildRequests
     * @property {string|null} [winnerHost] ResourceMetricsFrame winnerHost
     * @property {string|null} [slowestCircuit] ResourceMetricsFrame slowestCircuit
     * @property {number|null} [lateThrottles] ResourceMetricsFrame lateThrottles
     * @property {number|null} [outlierIsolations] ResourceMetricsFrame outlierIsolations
     * @property {number|null} [downloadHostCacheHits] ResourceMetricsFrame downloadHostCacheHits
     * @property {number|null} [downloadProbePromotionHits] ResourceMetricsFrame downloadProbePromotionHits
     * @property {number|null} [downloadLowSpeedAborts] ResourceMetricsFrame downloadLowSpeedAborts
     * @property {number|null} [downloadProbeQuarantineHits] ResourceMetricsFrame downloadProbeQuarantineHits
     * @property {number|null} [downloadProbeCandidateExhaustions] ResourceMetricsFrame downloadProbeCandidateExhaustions
     */

    /**
     * Constructs a new ResourceMetricsFrame.
     * @exports ResourceMetricsFrame
     * @classdesc Represents a ResourceMetricsFrame.
     * @implements IResourceMetricsFrame
     * @constructor
     * @param {IResourceMetricsFrame=} [properties] Properties to set
     */
    function ResourceMetricsFrame(properties) {
        if (properties)
            for (let keys = Object.keys(properties), i = 0; i < keys.length; ++i)
                if (properties[keys[i]] != null)
                    this[keys[i]] = properties[keys[i]];
    }

    /**
     * ResourceMetricsFrame processCpuPercent.
     * @member {number} processCpuPercent
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.processCpuPercent = 0;

    /**
     * ResourceMetricsFrame processMemoryBytes.
     * @member {number|Long} processMemoryBytes
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.processMemoryBytes = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * ResourceMetricsFrame systemMemoryUsedBytes.
     * @member {number|Long} systemMemoryUsedBytes
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.systemMemoryUsedBytes = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * ResourceMetricsFrame systemMemoryTotalBytes.
     * @member {number|Long} systemMemoryTotalBytes
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.systemMemoryTotalBytes = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * ResourceMetricsFrame activeWorkers.
     * @member {number} activeWorkers
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.activeWorkers = 0;

    /**
     * ResourceMetricsFrame workerTarget.
     * @member {number} workerTarget
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.workerTarget = 0;

    /**
     * ResourceMetricsFrame activeCircuits.
     * @member {number} activeCircuits
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.activeCircuits = 0;

    /**
     * ResourceMetricsFrame peakActiveCircuits.
     * @member {number} peakActiveCircuits
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.peakActiveCircuits = 0;

    /**
     * ResourceMetricsFrame currentNodeHost.
     * @member {string|null|undefined} currentNodeHost
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.currentNodeHost = null;

    /**
     * ResourceMetricsFrame nodeFailovers.
     * @member {number} nodeFailovers
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.nodeFailovers = 0;

    /**
     * ResourceMetricsFrame throttleCount.
     * @member {number} throttleCount
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.throttleCount = 0;

    /**
     * ResourceMetricsFrame timeoutCount.
     * @member {number} timeoutCount
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.timeoutCount = 0;

    /**
     * ResourceMetricsFrame throttleRatePerSec.
     * @member {number} throttleRatePerSec
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.throttleRatePerSec = 0;

    /**
     * ResourceMetricsFrame phantomPoolDepth.
     * @member {number} phantomPoolDepth
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.phantomPoolDepth = 0;

    /**
     * ResourceMetricsFrame subtreeReroutes.
     * @member {number} subtreeReroutes
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.subtreeReroutes = 0;

    /**
     * ResourceMetricsFrame subtreeQuarantineHits.
     * @member {number} subtreeQuarantineHits
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.subtreeQuarantineHits = 0;

    /**
     * ResourceMetricsFrame offWinnerChildRequests.
     * @member {number} offWinnerChildRequests
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.offWinnerChildRequests = 0;

    /**
     * ResourceMetricsFrame winnerHost.
     * @member {string|null|undefined} winnerHost
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.winnerHost = null;

    /**
     * ResourceMetricsFrame slowestCircuit.
     * @member {string|null|undefined} slowestCircuit
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.slowestCircuit = null;

    /**
     * ResourceMetricsFrame lateThrottles.
     * @member {number} lateThrottles
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.lateThrottles = 0;

    /**
     * ResourceMetricsFrame outlierIsolations.
     * @member {number} outlierIsolations
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.outlierIsolations = 0;

    /**
     * ResourceMetricsFrame downloadHostCacheHits.
     * @member {number} downloadHostCacheHits
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.downloadHostCacheHits = 0;

    /**
     * ResourceMetricsFrame downloadProbePromotionHits.
     * @member {number} downloadProbePromotionHits
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.downloadProbePromotionHits = 0;

    /**
     * ResourceMetricsFrame downloadLowSpeedAborts.
     * @member {number} downloadLowSpeedAborts
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.downloadLowSpeedAborts = 0;

    /**
     * ResourceMetricsFrame downloadProbeQuarantineHits.
     * @member {number} downloadProbeQuarantineHits
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.downloadProbeQuarantineHits = 0;

    /**
     * ResourceMetricsFrame downloadProbeCandidateExhaustions.
     * @member {number} downloadProbeCandidateExhaustions
     * @memberof ResourceMetricsFrame
     * @instance
     */
    ResourceMetricsFrame.prototype.downloadProbeCandidateExhaustions = 0;

    // OneOf field names bound to virtual getters and setters
    let $oneOfFields;

    // Virtual OneOf for proto3 optional field
    Object.defineProperty(ResourceMetricsFrame.prototype, "_currentNodeHost", {
        get: $util.oneOfGetter($oneOfFields = ["currentNodeHost"]),
        set: $util.oneOfSetter($oneOfFields)
    });

    // Virtual OneOf for proto3 optional field
    Object.defineProperty(ResourceMetricsFrame.prototype, "_winnerHost", {
        get: $util.oneOfGetter($oneOfFields = ["winnerHost"]),
        set: $util.oneOfSetter($oneOfFields)
    });

    // Virtual OneOf for proto3 optional field
    Object.defineProperty(ResourceMetricsFrame.prototype, "_slowestCircuit", {
        get: $util.oneOfGetter($oneOfFields = ["slowestCircuit"]),
        set: $util.oneOfSetter($oneOfFields)
    });

    /**
     * Creates a new ResourceMetricsFrame instance using the specified properties.
     * @function create
     * @memberof ResourceMetricsFrame
     * @static
     * @param {IResourceMetricsFrame=} [properties] Properties to set
     * @returns {ResourceMetricsFrame} ResourceMetricsFrame instance
     */
    ResourceMetricsFrame.create = function create(properties) {
        return new ResourceMetricsFrame(properties);
    };

    /**
     * Encodes the specified ResourceMetricsFrame message. Does not implicitly {@link ResourceMetricsFrame.verify|verify} messages.
     * @function encode
     * @memberof ResourceMetricsFrame
     * @static
     * @param {IResourceMetricsFrame} message ResourceMetricsFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    ResourceMetricsFrame.encode = function encode(message, writer) {
        if (!writer)
            writer = $Writer.create();
        if (message.processCpuPercent != null && Object.hasOwnProperty.call(message, "processCpuPercent"))
            writer.uint32(/* id 1, wireType 1 =*/9).double(message.processCpuPercent);
        if (message.processMemoryBytes != null && Object.hasOwnProperty.call(message, "processMemoryBytes"))
            writer.uint32(/* id 2, wireType 0 =*/16).uint64(message.processMemoryBytes);
        if (message.systemMemoryUsedBytes != null && Object.hasOwnProperty.call(message, "systemMemoryUsedBytes"))
            writer.uint32(/* id 3, wireType 0 =*/24).uint64(message.systemMemoryUsedBytes);
        if (message.systemMemoryTotalBytes != null && Object.hasOwnProperty.call(message, "systemMemoryTotalBytes"))
            writer.uint32(/* id 4, wireType 0 =*/32).uint64(message.systemMemoryTotalBytes);
        if (message.activeWorkers != null && Object.hasOwnProperty.call(message, "activeWorkers"))
            writer.uint32(/* id 5, wireType 0 =*/40).uint32(message.activeWorkers);
        if (message.workerTarget != null && Object.hasOwnProperty.call(message, "workerTarget"))
            writer.uint32(/* id 6, wireType 0 =*/48).uint32(message.workerTarget);
        if (message.activeCircuits != null && Object.hasOwnProperty.call(message, "activeCircuits"))
            writer.uint32(/* id 7, wireType 0 =*/56).uint32(message.activeCircuits);
        if (message.peakActiveCircuits != null && Object.hasOwnProperty.call(message, "peakActiveCircuits"))
            writer.uint32(/* id 8, wireType 0 =*/64).uint32(message.peakActiveCircuits);
        if (message.currentNodeHost != null && Object.hasOwnProperty.call(message, "currentNodeHost"))
            writer.uint32(/* id 9, wireType 2 =*/74).string(message.currentNodeHost);
        if (message.nodeFailovers != null && Object.hasOwnProperty.call(message, "nodeFailovers"))
            writer.uint32(/* id 10, wireType 0 =*/80).uint32(message.nodeFailovers);
        if (message.throttleCount != null && Object.hasOwnProperty.call(message, "throttleCount"))
            writer.uint32(/* id 11, wireType 0 =*/88).uint32(message.throttleCount);
        if (message.timeoutCount != null && Object.hasOwnProperty.call(message, "timeoutCount"))
            writer.uint32(/* id 12, wireType 0 =*/96).uint32(message.timeoutCount);
        if (message.throttleRatePerSec != null && Object.hasOwnProperty.call(message, "throttleRatePerSec"))
            writer.uint32(/* id 13, wireType 1 =*/105).double(message.throttleRatePerSec);
        if (message.phantomPoolDepth != null && Object.hasOwnProperty.call(message, "phantomPoolDepth"))
            writer.uint32(/* id 14, wireType 0 =*/112).uint32(message.phantomPoolDepth);
        if (message.subtreeReroutes != null && Object.hasOwnProperty.call(message, "subtreeReroutes"))
            writer.uint32(/* id 15, wireType 0 =*/120).uint32(message.subtreeReroutes);
        if (message.subtreeQuarantineHits != null && Object.hasOwnProperty.call(message, "subtreeQuarantineHits"))
            writer.uint32(/* id 16, wireType 0 =*/128).uint32(message.subtreeQuarantineHits);
        if (message.offWinnerChildRequests != null && Object.hasOwnProperty.call(message, "offWinnerChildRequests"))
            writer.uint32(/* id 17, wireType 0 =*/136).uint32(message.offWinnerChildRequests);
        if (message.winnerHost != null && Object.hasOwnProperty.call(message, "winnerHost"))
            writer.uint32(/* id 18, wireType 2 =*/146).string(message.winnerHost);
        if (message.slowestCircuit != null && Object.hasOwnProperty.call(message, "slowestCircuit"))
            writer.uint32(/* id 19, wireType 2 =*/154).string(message.slowestCircuit);
        if (message.lateThrottles != null && Object.hasOwnProperty.call(message, "lateThrottles"))
            writer.uint32(/* id 20, wireType 0 =*/160).uint32(message.lateThrottles);
        if (message.outlierIsolations != null && Object.hasOwnProperty.call(message, "outlierIsolations"))
            writer.uint32(/* id 21, wireType 0 =*/168).uint32(message.outlierIsolations);
        if (message.downloadHostCacheHits != null && Object.hasOwnProperty.call(message, "downloadHostCacheHits"))
            writer.uint32(/* id 22, wireType 0 =*/176).uint32(message.downloadHostCacheHits);
        if (message.downloadProbePromotionHits != null && Object.hasOwnProperty.call(message, "downloadProbePromotionHits"))
            writer.uint32(/* id 23, wireType 0 =*/184).uint32(message.downloadProbePromotionHits);
        if (message.downloadLowSpeedAborts != null && Object.hasOwnProperty.call(message, "downloadLowSpeedAborts"))
            writer.uint32(/* id 24, wireType 0 =*/192).uint32(message.downloadLowSpeedAborts);
        if (message.downloadProbeQuarantineHits != null && Object.hasOwnProperty.call(message, "downloadProbeQuarantineHits"))
            writer.uint32(/* id 25, wireType 0 =*/200).uint32(message.downloadProbeQuarantineHits);
        if (message.downloadProbeCandidateExhaustions != null && Object.hasOwnProperty.call(message, "downloadProbeCandidateExhaustions"))
            writer.uint32(/* id 26, wireType 0 =*/208).uint32(message.downloadProbeCandidateExhaustions);
        return writer;
    };

    /**
     * Encodes the specified ResourceMetricsFrame message, length delimited. Does not implicitly {@link ResourceMetricsFrame.verify|verify} messages.
     * @function encodeDelimited
     * @memberof ResourceMetricsFrame
     * @static
     * @param {IResourceMetricsFrame} message ResourceMetricsFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    ResourceMetricsFrame.encodeDelimited = function encodeDelimited(message, writer) {
        return this.encode(message, writer).ldelim();
    };

    /**
     * Decodes a ResourceMetricsFrame message from the specified reader or buffer.
     * @function decode
     * @memberof ResourceMetricsFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @param {number} [length] Message length if known beforehand
     * @returns {ResourceMetricsFrame} ResourceMetricsFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    ResourceMetricsFrame.decode = function decode(reader, length, error) {
        if (!(reader instanceof $Reader))
            reader = $Reader.create(reader);
        let end = length === undefined ? reader.len : reader.pos + length, message = new $root.ResourceMetricsFrame();
        while (reader.pos < end) {
            let tag = reader.uint32();
            if (tag === error)
                break;
            switch (tag >>> 3) {
            case 1: {
                    message.processCpuPercent = reader.double();
                    break;
                }
            case 2: {
                    message.processMemoryBytes = reader.uint64();
                    break;
                }
            case 3: {
                    message.systemMemoryUsedBytes = reader.uint64();
                    break;
                }
            case 4: {
                    message.systemMemoryTotalBytes = reader.uint64();
                    break;
                }
            case 5: {
                    message.activeWorkers = reader.uint32();
                    break;
                }
            case 6: {
                    message.workerTarget = reader.uint32();
                    break;
                }
            case 7: {
                    message.activeCircuits = reader.uint32();
                    break;
                }
            case 8: {
                    message.peakActiveCircuits = reader.uint32();
                    break;
                }
            case 9: {
                    message.currentNodeHost = reader.string();
                    break;
                }
            case 10: {
                    message.nodeFailovers = reader.uint32();
                    break;
                }
            case 11: {
                    message.throttleCount = reader.uint32();
                    break;
                }
            case 12: {
                    message.timeoutCount = reader.uint32();
                    break;
                }
            case 13: {
                    message.throttleRatePerSec = reader.double();
                    break;
                }
            case 14: {
                    message.phantomPoolDepth = reader.uint32();
                    break;
                }
            case 15: {
                    message.subtreeReroutes = reader.uint32();
                    break;
                }
            case 16: {
                    message.subtreeQuarantineHits = reader.uint32();
                    break;
                }
            case 17: {
                    message.offWinnerChildRequests = reader.uint32();
                    break;
                }
            case 18: {
                    message.winnerHost = reader.string();
                    break;
                }
            case 19: {
                    message.slowestCircuit = reader.string();
                    break;
                }
            case 20: {
                    message.lateThrottles = reader.uint32();
                    break;
                }
            case 21: {
                    message.outlierIsolations = reader.uint32();
                    break;
                }
            case 22: {
                    message.downloadHostCacheHits = reader.uint32();
                    break;
                }
            case 23: {
                    message.downloadProbePromotionHits = reader.uint32();
                    break;
                }
            case 24: {
                    message.downloadLowSpeedAborts = reader.uint32();
                    break;
                }
            case 25: {
                    message.downloadProbeQuarantineHits = reader.uint32();
                    break;
                }
            case 26: {
                    message.downloadProbeCandidateExhaustions = reader.uint32();
                    break;
                }
            default:
                reader.skipType(tag & 7);
                break;
            }
        }
        return message;
    };

    /**
     * Decodes a ResourceMetricsFrame message from the specified reader or buffer, length delimited.
     * @function decodeDelimited
     * @memberof ResourceMetricsFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @returns {ResourceMetricsFrame} ResourceMetricsFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    ResourceMetricsFrame.decodeDelimited = function decodeDelimited(reader) {
        if (!(reader instanceof $Reader))
            reader = new $Reader(reader);
        return this.decode(reader, reader.uint32());
    };

    /**
     * Verifies a ResourceMetricsFrame message.
     * @function verify
     * @memberof ResourceMetricsFrame
     * @static
     * @param {Object.<string,*>} message Plain object to verify
     * @returns {string|null} `null` if valid, otherwise the reason why it is not
     */
    ResourceMetricsFrame.verify = function verify(message) {
        if (typeof message !== "object" || message === null)
            return "object expected";
        let properties = {};
        if (message.processCpuPercent != null && message.hasOwnProperty("processCpuPercent"))
            if (typeof message.processCpuPercent !== "number")
                return "processCpuPercent: number expected";
        if (message.processMemoryBytes != null && message.hasOwnProperty("processMemoryBytes"))
            if (!$util.isInteger(message.processMemoryBytes) && !(message.processMemoryBytes && $util.isInteger(message.processMemoryBytes.low) && $util.isInteger(message.processMemoryBytes.high)))
                return "processMemoryBytes: integer|Long expected";
        if (message.systemMemoryUsedBytes != null && message.hasOwnProperty("systemMemoryUsedBytes"))
            if (!$util.isInteger(message.systemMemoryUsedBytes) && !(message.systemMemoryUsedBytes && $util.isInteger(message.systemMemoryUsedBytes.low) && $util.isInteger(message.systemMemoryUsedBytes.high)))
                return "systemMemoryUsedBytes: integer|Long expected";
        if (message.systemMemoryTotalBytes != null && message.hasOwnProperty("systemMemoryTotalBytes"))
            if (!$util.isInteger(message.systemMemoryTotalBytes) && !(message.systemMemoryTotalBytes && $util.isInteger(message.systemMemoryTotalBytes.low) && $util.isInteger(message.systemMemoryTotalBytes.high)))
                return "systemMemoryTotalBytes: integer|Long expected";
        if (message.activeWorkers != null && message.hasOwnProperty("activeWorkers"))
            if (!$util.isInteger(message.activeWorkers))
                return "activeWorkers: integer expected";
        if (message.workerTarget != null && message.hasOwnProperty("workerTarget"))
            if (!$util.isInteger(message.workerTarget))
                return "workerTarget: integer expected";
        if (message.activeCircuits != null && message.hasOwnProperty("activeCircuits"))
            if (!$util.isInteger(message.activeCircuits))
                return "activeCircuits: integer expected";
        if (message.peakActiveCircuits != null && message.hasOwnProperty("peakActiveCircuits"))
            if (!$util.isInteger(message.peakActiveCircuits))
                return "peakActiveCircuits: integer expected";
        if (message.currentNodeHost != null && message.hasOwnProperty("currentNodeHost")) {
            properties._currentNodeHost = 1;
            if (!$util.isString(message.currentNodeHost))
                return "currentNodeHost: string expected";
        }
        if (message.nodeFailovers != null && message.hasOwnProperty("nodeFailovers"))
            if (!$util.isInteger(message.nodeFailovers))
                return "nodeFailovers: integer expected";
        if (message.throttleCount != null && message.hasOwnProperty("throttleCount"))
            if (!$util.isInteger(message.throttleCount))
                return "throttleCount: integer expected";
        if (message.timeoutCount != null && message.hasOwnProperty("timeoutCount"))
            if (!$util.isInteger(message.timeoutCount))
                return "timeoutCount: integer expected";
        if (message.throttleRatePerSec != null && message.hasOwnProperty("throttleRatePerSec"))
            if (typeof message.throttleRatePerSec !== "number")
                return "throttleRatePerSec: number expected";
        if (message.phantomPoolDepth != null && message.hasOwnProperty("phantomPoolDepth"))
            if (!$util.isInteger(message.phantomPoolDepth))
                return "phantomPoolDepth: integer expected";
        if (message.subtreeReroutes != null && message.hasOwnProperty("subtreeReroutes"))
            if (!$util.isInteger(message.subtreeReroutes))
                return "subtreeReroutes: integer expected";
        if (message.subtreeQuarantineHits != null && message.hasOwnProperty("subtreeQuarantineHits"))
            if (!$util.isInteger(message.subtreeQuarantineHits))
                return "subtreeQuarantineHits: integer expected";
        if (message.offWinnerChildRequests != null && message.hasOwnProperty("offWinnerChildRequests"))
            if (!$util.isInteger(message.offWinnerChildRequests))
                return "offWinnerChildRequests: integer expected";
        if (message.winnerHost != null && message.hasOwnProperty("winnerHost")) {
            properties._winnerHost = 1;
            if (!$util.isString(message.winnerHost))
                return "winnerHost: string expected";
        }
        if (message.slowestCircuit != null && message.hasOwnProperty("slowestCircuit")) {
            properties._slowestCircuit = 1;
            if (!$util.isString(message.slowestCircuit))
                return "slowestCircuit: string expected";
        }
        if (message.lateThrottles != null && message.hasOwnProperty("lateThrottles"))
            if (!$util.isInteger(message.lateThrottles))
                return "lateThrottles: integer expected";
        if (message.outlierIsolations != null && message.hasOwnProperty("outlierIsolations"))
            if (!$util.isInteger(message.outlierIsolations))
                return "outlierIsolations: integer expected";
        if (message.downloadHostCacheHits != null && message.hasOwnProperty("downloadHostCacheHits"))
            if (!$util.isInteger(message.downloadHostCacheHits))
                return "downloadHostCacheHits: integer expected";
        if (message.downloadProbePromotionHits != null && message.hasOwnProperty("downloadProbePromotionHits"))
            if (!$util.isInteger(message.downloadProbePromotionHits))
                return "downloadProbePromotionHits: integer expected";
        if (message.downloadLowSpeedAborts != null && message.hasOwnProperty("downloadLowSpeedAborts"))
            if (!$util.isInteger(message.downloadLowSpeedAborts))
                return "downloadLowSpeedAborts: integer expected";
        if (message.downloadProbeQuarantineHits != null && message.hasOwnProperty("downloadProbeQuarantineHits"))
            if (!$util.isInteger(message.downloadProbeQuarantineHits))
                return "downloadProbeQuarantineHits: integer expected";
        if (message.downloadProbeCandidateExhaustions != null && message.hasOwnProperty("downloadProbeCandidateExhaustions"))
            if (!$util.isInteger(message.downloadProbeCandidateExhaustions))
                return "downloadProbeCandidateExhaustions: integer expected";
        return null;
    };

    /**
     * Creates a ResourceMetricsFrame message from a plain object. Also converts values to their respective internal types.
     * @function fromObject
     * @memberof ResourceMetricsFrame
     * @static
     * @param {Object.<string,*>} object Plain object
     * @returns {ResourceMetricsFrame} ResourceMetricsFrame
     */
    ResourceMetricsFrame.fromObject = function fromObject(object) {
        if (object instanceof $root.ResourceMetricsFrame)
            return object;
        let message = new $root.ResourceMetricsFrame();
        if (object.processCpuPercent != null)
            message.processCpuPercent = Number(object.processCpuPercent);
        if (object.processMemoryBytes != null)
            if ($util.Long)
                (message.processMemoryBytes = $util.Long.fromValue(object.processMemoryBytes)).unsigned = true;
            else if (typeof object.processMemoryBytes === "string")
                message.processMemoryBytes = parseInt(object.processMemoryBytes, 10);
            else if (typeof object.processMemoryBytes === "number")
                message.processMemoryBytes = object.processMemoryBytes;
            else if (typeof object.processMemoryBytes === "object")
                message.processMemoryBytes = new $util.LongBits(object.processMemoryBytes.low >>> 0, object.processMemoryBytes.high >>> 0).toNumber(true);
        if (object.systemMemoryUsedBytes != null)
            if ($util.Long)
                (message.systemMemoryUsedBytes = $util.Long.fromValue(object.systemMemoryUsedBytes)).unsigned = true;
            else if (typeof object.systemMemoryUsedBytes === "string")
                message.systemMemoryUsedBytes = parseInt(object.systemMemoryUsedBytes, 10);
            else if (typeof object.systemMemoryUsedBytes === "number")
                message.systemMemoryUsedBytes = object.systemMemoryUsedBytes;
            else if (typeof object.systemMemoryUsedBytes === "object")
                message.systemMemoryUsedBytes = new $util.LongBits(object.systemMemoryUsedBytes.low >>> 0, object.systemMemoryUsedBytes.high >>> 0).toNumber(true);
        if (object.systemMemoryTotalBytes != null)
            if ($util.Long)
                (message.systemMemoryTotalBytes = $util.Long.fromValue(object.systemMemoryTotalBytes)).unsigned = true;
            else if (typeof object.systemMemoryTotalBytes === "string")
                message.systemMemoryTotalBytes = parseInt(object.systemMemoryTotalBytes, 10);
            else if (typeof object.systemMemoryTotalBytes === "number")
                message.systemMemoryTotalBytes = object.systemMemoryTotalBytes;
            else if (typeof object.systemMemoryTotalBytes === "object")
                message.systemMemoryTotalBytes = new $util.LongBits(object.systemMemoryTotalBytes.low >>> 0, object.systemMemoryTotalBytes.high >>> 0).toNumber(true);
        if (object.activeWorkers != null)
            message.activeWorkers = object.activeWorkers >>> 0;
        if (object.workerTarget != null)
            message.workerTarget = object.workerTarget >>> 0;
        if (object.activeCircuits != null)
            message.activeCircuits = object.activeCircuits >>> 0;
        if (object.peakActiveCircuits != null)
            message.peakActiveCircuits = object.peakActiveCircuits >>> 0;
        if (object.currentNodeHost != null)
            message.currentNodeHost = String(object.currentNodeHost);
        if (object.nodeFailovers != null)
            message.nodeFailovers = object.nodeFailovers >>> 0;
        if (object.throttleCount != null)
            message.throttleCount = object.throttleCount >>> 0;
        if (object.timeoutCount != null)
            message.timeoutCount = object.timeoutCount >>> 0;
        if (object.throttleRatePerSec != null)
            message.throttleRatePerSec = Number(object.throttleRatePerSec);
        if (object.phantomPoolDepth != null)
            message.phantomPoolDepth = object.phantomPoolDepth >>> 0;
        if (object.subtreeReroutes != null)
            message.subtreeReroutes = object.subtreeReroutes >>> 0;
        if (object.subtreeQuarantineHits != null)
            message.subtreeQuarantineHits = object.subtreeQuarantineHits >>> 0;
        if (object.offWinnerChildRequests != null)
            message.offWinnerChildRequests = object.offWinnerChildRequests >>> 0;
        if (object.winnerHost != null)
            message.winnerHost = String(object.winnerHost);
        if (object.slowestCircuit != null)
            message.slowestCircuit = String(object.slowestCircuit);
        if (object.lateThrottles != null)
            message.lateThrottles = object.lateThrottles >>> 0;
        if (object.outlierIsolations != null)
            message.outlierIsolations = object.outlierIsolations >>> 0;
        if (object.downloadHostCacheHits != null)
            message.downloadHostCacheHits = object.downloadHostCacheHits >>> 0;
        if (object.downloadProbePromotionHits != null)
            message.downloadProbePromotionHits = object.downloadProbePromotionHits >>> 0;
        if (object.downloadLowSpeedAborts != null)
            message.downloadLowSpeedAborts = object.downloadLowSpeedAborts >>> 0;
        if (object.downloadProbeQuarantineHits != null)
            message.downloadProbeQuarantineHits = object.downloadProbeQuarantineHits >>> 0;
        if (object.downloadProbeCandidateExhaustions != null)
            message.downloadProbeCandidateExhaustions = object.downloadProbeCandidateExhaustions >>> 0;
        return message;
    };

    /**
     * Creates a plain object from a ResourceMetricsFrame message. Also converts values to other types if specified.
     * @function toObject
     * @memberof ResourceMetricsFrame
     * @static
     * @param {ResourceMetricsFrame} message ResourceMetricsFrame
     * @param {$protobuf.IConversionOptions} [options] Conversion options
     * @returns {Object.<string,*>} Plain object
     */
    ResourceMetricsFrame.toObject = function toObject(message, options) {
        if (!options)
            options = {};
        let object = {};
        if (options.defaults) {
            object.processCpuPercent = 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.processMemoryBytes = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.processMemoryBytes = options.longs === String ? "0" : 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.systemMemoryUsedBytes = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.systemMemoryUsedBytes = options.longs === String ? "0" : 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.systemMemoryTotalBytes = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.systemMemoryTotalBytes = options.longs === String ? "0" : 0;
            object.activeWorkers = 0;
            object.workerTarget = 0;
            object.activeCircuits = 0;
            object.peakActiveCircuits = 0;
            object.nodeFailovers = 0;
            object.throttleCount = 0;
            object.timeoutCount = 0;
            object.throttleRatePerSec = 0;
            object.phantomPoolDepth = 0;
            object.subtreeReroutes = 0;
            object.subtreeQuarantineHits = 0;
            object.offWinnerChildRequests = 0;
            object.lateThrottles = 0;
            object.outlierIsolations = 0;
            object.downloadHostCacheHits = 0;
            object.downloadProbePromotionHits = 0;
            object.downloadLowSpeedAborts = 0;
            object.downloadProbeQuarantineHits = 0;
            object.downloadProbeCandidateExhaustions = 0;
        }
        if (message.processCpuPercent != null && message.hasOwnProperty("processCpuPercent"))
            object.processCpuPercent = options.json && !isFinite(message.processCpuPercent) ? String(message.processCpuPercent) : message.processCpuPercent;
        if (message.processMemoryBytes != null && message.hasOwnProperty("processMemoryBytes"))
            if (typeof message.processMemoryBytes === "number")
                object.processMemoryBytes = options.longs === String ? String(message.processMemoryBytes) : message.processMemoryBytes;
            else
                object.processMemoryBytes = options.longs === String ? $util.Long.prototype.toString.call(message.processMemoryBytes) : options.longs === Number ? new $util.LongBits(message.processMemoryBytes.low >>> 0, message.processMemoryBytes.high >>> 0).toNumber(true) : message.processMemoryBytes;
        if (message.systemMemoryUsedBytes != null && message.hasOwnProperty("systemMemoryUsedBytes"))
            if (typeof message.systemMemoryUsedBytes === "number")
                object.systemMemoryUsedBytes = options.longs === String ? String(message.systemMemoryUsedBytes) : message.systemMemoryUsedBytes;
            else
                object.systemMemoryUsedBytes = options.longs === String ? $util.Long.prototype.toString.call(message.systemMemoryUsedBytes) : options.longs === Number ? new $util.LongBits(message.systemMemoryUsedBytes.low >>> 0, message.systemMemoryUsedBytes.high >>> 0).toNumber(true) : message.systemMemoryUsedBytes;
        if (message.systemMemoryTotalBytes != null && message.hasOwnProperty("systemMemoryTotalBytes"))
            if (typeof message.systemMemoryTotalBytes === "number")
                object.systemMemoryTotalBytes = options.longs === String ? String(message.systemMemoryTotalBytes) : message.systemMemoryTotalBytes;
            else
                object.systemMemoryTotalBytes = options.longs === String ? $util.Long.prototype.toString.call(message.systemMemoryTotalBytes) : options.longs === Number ? new $util.LongBits(message.systemMemoryTotalBytes.low >>> 0, message.systemMemoryTotalBytes.high >>> 0).toNumber(true) : message.systemMemoryTotalBytes;
        if (message.activeWorkers != null && message.hasOwnProperty("activeWorkers"))
            object.activeWorkers = message.activeWorkers;
        if (message.workerTarget != null && message.hasOwnProperty("workerTarget"))
            object.workerTarget = message.workerTarget;
        if (message.activeCircuits != null && message.hasOwnProperty("activeCircuits"))
            object.activeCircuits = message.activeCircuits;
        if (message.peakActiveCircuits != null && message.hasOwnProperty("peakActiveCircuits"))
            object.peakActiveCircuits = message.peakActiveCircuits;
        if (message.currentNodeHost != null && message.hasOwnProperty("currentNodeHost")) {
            object.currentNodeHost = message.currentNodeHost;
            if (options.oneofs)
                object._currentNodeHost = "currentNodeHost";
        }
        if (message.nodeFailovers != null && message.hasOwnProperty("nodeFailovers"))
            object.nodeFailovers = message.nodeFailovers;
        if (message.throttleCount != null && message.hasOwnProperty("throttleCount"))
            object.throttleCount = message.throttleCount;
        if (message.timeoutCount != null && message.hasOwnProperty("timeoutCount"))
            object.timeoutCount = message.timeoutCount;
        if (message.throttleRatePerSec != null && message.hasOwnProperty("throttleRatePerSec"))
            object.throttleRatePerSec = options.json && !isFinite(message.throttleRatePerSec) ? String(message.throttleRatePerSec) : message.throttleRatePerSec;
        if (message.phantomPoolDepth != null && message.hasOwnProperty("phantomPoolDepth"))
            object.phantomPoolDepth = message.phantomPoolDepth;
        if (message.subtreeReroutes != null && message.hasOwnProperty("subtreeReroutes"))
            object.subtreeReroutes = message.subtreeReroutes;
        if (message.subtreeQuarantineHits != null && message.hasOwnProperty("subtreeQuarantineHits"))
            object.subtreeQuarantineHits = message.subtreeQuarantineHits;
        if (message.offWinnerChildRequests != null && message.hasOwnProperty("offWinnerChildRequests"))
            object.offWinnerChildRequests = message.offWinnerChildRequests;
        if (message.winnerHost != null && message.hasOwnProperty("winnerHost")) {
            object.winnerHost = message.winnerHost;
            if (options.oneofs)
                object._winnerHost = "winnerHost";
        }
        if (message.slowestCircuit != null && message.hasOwnProperty("slowestCircuit")) {
            object.slowestCircuit = message.slowestCircuit;
            if (options.oneofs)
                object._slowestCircuit = "slowestCircuit";
        }
        if (message.lateThrottles != null && message.hasOwnProperty("lateThrottles"))
            object.lateThrottles = message.lateThrottles;
        if (message.outlierIsolations != null && message.hasOwnProperty("outlierIsolations"))
            object.outlierIsolations = message.outlierIsolations;
        if (message.downloadHostCacheHits != null && message.hasOwnProperty("downloadHostCacheHits"))
            object.downloadHostCacheHits = message.downloadHostCacheHits;
        if (message.downloadProbePromotionHits != null && message.hasOwnProperty("downloadProbePromotionHits"))
            object.downloadProbePromotionHits = message.downloadProbePromotionHits;
        if (message.downloadLowSpeedAborts != null && message.hasOwnProperty("downloadLowSpeedAborts"))
            object.downloadLowSpeedAborts = message.downloadLowSpeedAborts;
        if (message.downloadProbeQuarantineHits != null && message.hasOwnProperty("downloadProbeQuarantineHits"))
            object.downloadProbeQuarantineHits = message.downloadProbeQuarantineHits;
        if (message.downloadProbeCandidateExhaustions != null && message.hasOwnProperty("downloadProbeCandidateExhaustions"))
            object.downloadProbeCandidateExhaustions = message.downloadProbeCandidateExhaustions;
        return object;
    };

    /**
     * Converts this ResourceMetricsFrame to JSON.
     * @function toJSON
     * @memberof ResourceMetricsFrame
     * @instance
     * @returns {Object.<string,*>} JSON object
     */
    ResourceMetricsFrame.prototype.toJSON = function toJSON() {
        return this.constructor.toObject(this, $protobuf.util.toJSONOptions);
    };

    /**
     * Gets the default type url for ResourceMetricsFrame
     * @function getTypeUrl
     * @memberof ResourceMetricsFrame
     * @static
     * @param {string} [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns {string} The default type url
     */
    ResourceMetricsFrame.getTypeUrl = function getTypeUrl(typeUrlPrefix) {
        if (typeUrlPrefix === undefined) {
            typeUrlPrefix = "type.googleapis.com";
        }
        return typeUrlPrefix + "/ResourceMetricsFrame";
    };

    return ResourceMetricsFrame;
})();

export const CrawlStatusFrame = $root.CrawlStatusFrame = (() => {

    /**
     * Properties of a CrawlStatusFrame.
     * @exports ICrawlStatusFrame
     * @interface ICrawlStatusFrame
     * @property {string|null} [phase] CrawlStatusFrame phase
     * @property {number|null} [progressPercent] CrawlStatusFrame progressPercent
     * @property {number|Long|null} [visitedNodes] CrawlStatusFrame visitedNodes
     * @property {number|Long|null} [processedNodes] CrawlStatusFrame processedNodes
     * @property {number|Long|null} [queuedNodes] CrawlStatusFrame queuedNodes
     * @property {number|null} [activeWorkers] CrawlStatusFrame activeWorkers
     * @property {number|null} [workerTarget] CrawlStatusFrame workerTarget
     * @property {number|Long|null} [etaSeconds] CrawlStatusFrame etaSeconds
     * @property {number|Long|null} [deltaNewFiles] CrawlStatusFrame deltaNewFiles
     */

    /**
     * Constructs a new CrawlStatusFrame.
     * @exports CrawlStatusFrame
     * @classdesc Represents a CrawlStatusFrame.
     * @implements ICrawlStatusFrame
     * @constructor
     * @param {ICrawlStatusFrame=} [properties] Properties to set
     */
    function CrawlStatusFrame(properties) {
        if (properties)
            for (let keys = Object.keys(properties), i = 0; i < keys.length; ++i)
                if (properties[keys[i]] != null)
                    this[keys[i]] = properties[keys[i]];
    }

    /**
     * CrawlStatusFrame phase.
     * @member {string} phase
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.phase = "";

    /**
     * CrawlStatusFrame progressPercent.
     * @member {number} progressPercent
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.progressPercent = 0;

    /**
     * CrawlStatusFrame visitedNodes.
     * @member {number|Long} visitedNodes
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.visitedNodes = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * CrawlStatusFrame processedNodes.
     * @member {number|Long} processedNodes
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.processedNodes = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * CrawlStatusFrame queuedNodes.
     * @member {number|Long} queuedNodes
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.queuedNodes = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * CrawlStatusFrame activeWorkers.
     * @member {number} activeWorkers
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.activeWorkers = 0;

    /**
     * CrawlStatusFrame workerTarget.
     * @member {number} workerTarget
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.workerTarget = 0;

    /**
     * CrawlStatusFrame etaSeconds.
     * @member {number|Long|null|undefined} etaSeconds
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.etaSeconds = null;

    /**
     * CrawlStatusFrame deltaNewFiles.
     * @member {number|Long} deltaNewFiles
     * @memberof CrawlStatusFrame
     * @instance
     */
    CrawlStatusFrame.prototype.deltaNewFiles = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    // OneOf field names bound to virtual getters and setters
    let $oneOfFields;

    // Virtual OneOf for proto3 optional field
    Object.defineProperty(CrawlStatusFrame.prototype, "_etaSeconds", {
        get: $util.oneOfGetter($oneOfFields = ["etaSeconds"]),
        set: $util.oneOfSetter($oneOfFields)
    });

    /**
     * Creates a new CrawlStatusFrame instance using the specified properties.
     * @function create
     * @memberof CrawlStatusFrame
     * @static
     * @param {ICrawlStatusFrame=} [properties] Properties to set
     * @returns {CrawlStatusFrame} CrawlStatusFrame instance
     */
    CrawlStatusFrame.create = function create(properties) {
        return new CrawlStatusFrame(properties);
    };

    /**
     * Encodes the specified CrawlStatusFrame message. Does not implicitly {@link CrawlStatusFrame.verify|verify} messages.
     * @function encode
     * @memberof CrawlStatusFrame
     * @static
     * @param {ICrawlStatusFrame} message CrawlStatusFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    CrawlStatusFrame.encode = function encode(message, writer) {
        if (!writer)
            writer = $Writer.create();
        if (message.phase != null && Object.hasOwnProperty.call(message, "phase"))
            writer.uint32(/* id 1, wireType 2 =*/10).string(message.phase);
        if (message.progressPercent != null && Object.hasOwnProperty.call(message, "progressPercent"))
            writer.uint32(/* id 2, wireType 1 =*/17).double(message.progressPercent);
        if (message.visitedNodes != null && Object.hasOwnProperty.call(message, "visitedNodes"))
            writer.uint32(/* id 3, wireType 0 =*/24).uint64(message.visitedNodes);
        if (message.processedNodes != null && Object.hasOwnProperty.call(message, "processedNodes"))
            writer.uint32(/* id 4, wireType 0 =*/32).uint64(message.processedNodes);
        if (message.queuedNodes != null && Object.hasOwnProperty.call(message, "queuedNodes"))
            writer.uint32(/* id 5, wireType 0 =*/40).uint64(message.queuedNodes);
        if (message.activeWorkers != null && Object.hasOwnProperty.call(message, "activeWorkers"))
            writer.uint32(/* id 6, wireType 0 =*/48).uint32(message.activeWorkers);
        if (message.workerTarget != null && Object.hasOwnProperty.call(message, "workerTarget"))
            writer.uint32(/* id 7, wireType 0 =*/56).uint32(message.workerTarget);
        if (message.etaSeconds != null && Object.hasOwnProperty.call(message, "etaSeconds"))
            writer.uint32(/* id 8, wireType 0 =*/64).uint64(message.etaSeconds);
        if (message.deltaNewFiles != null && Object.hasOwnProperty.call(message, "deltaNewFiles"))
            writer.uint32(/* id 9, wireType 0 =*/72).uint64(message.deltaNewFiles);
        return writer;
    };

    /**
     * Encodes the specified CrawlStatusFrame message, length delimited. Does not implicitly {@link CrawlStatusFrame.verify|verify} messages.
     * @function encodeDelimited
     * @memberof CrawlStatusFrame
     * @static
     * @param {ICrawlStatusFrame} message CrawlStatusFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    CrawlStatusFrame.encodeDelimited = function encodeDelimited(message, writer) {
        return this.encode(message, writer).ldelim();
    };

    /**
     * Decodes a CrawlStatusFrame message from the specified reader or buffer.
     * @function decode
     * @memberof CrawlStatusFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @param {number} [length] Message length if known beforehand
     * @returns {CrawlStatusFrame} CrawlStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    CrawlStatusFrame.decode = function decode(reader, length, error) {
        if (!(reader instanceof $Reader))
            reader = $Reader.create(reader);
        let end = length === undefined ? reader.len : reader.pos + length, message = new $root.CrawlStatusFrame();
        while (reader.pos < end) {
            let tag = reader.uint32();
            if (tag === error)
                break;
            switch (tag >>> 3) {
            case 1: {
                    message.phase = reader.string();
                    break;
                }
            case 2: {
                    message.progressPercent = reader.double();
                    break;
                }
            case 3: {
                    message.visitedNodes = reader.uint64();
                    break;
                }
            case 4: {
                    message.processedNodes = reader.uint64();
                    break;
                }
            case 5: {
                    message.queuedNodes = reader.uint64();
                    break;
                }
            case 6: {
                    message.activeWorkers = reader.uint32();
                    break;
                }
            case 7: {
                    message.workerTarget = reader.uint32();
                    break;
                }
            case 8: {
                    message.etaSeconds = reader.uint64();
                    break;
                }
            case 9: {
                    message.deltaNewFiles = reader.uint64();
                    break;
                }
            default:
                reader.skipType(tag & 7);
                break;
            }
        }
        return message;
    };

    /**
     * Decodes a CrawlStatusFrame message from the specified reader or buffer, length delimited.
     * @function decodeDelimited
     * @memberof CrawlStatusFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @returns {CrawlStatusFrame} CrawlStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    CrawlStatusFrame.decodeDelimited = function decodeDelimited(reader) {
        if (!(reader instanceof $Reader))
            reader = new $Reader(reader);
        return this.decode(reader, reader.uint32());
    };

    /**
     * Verifies a CrawlStatusFrame message.
     * @function verify
     * @memberof CrawlStatusFrame
     * @static
     * @param {Object.<string,*>} message Plain object to verify
     * @returns {string|null} `null` if valid, otherwise the reason why it is not
     */
    CrawlStatusFrame.verify = function verify(message) {
        if (typeof message !== "object" || message === null)
            return "object expected";
        let properties = {};
        if (message.phase != null && message.hasOwnProperty("phase"))
            if (!$util.isString(message.phase))
                return "phase: string expected";
        if (message.progressPercent != null && message.hasOwnProperty("progressPercent"))
            if (typeof message.progressPercent !== "number")
                return "progressPercent: number expected";
        if (message.visitedNodes != null && message.hasOwnProperty("visitedNodes"))
            if (!$util.isInteger(message.visitedNodes) && !(message.visitedNodes && $util.isInteger(message.visitedNodes.low) && $util.isInteger(message.visitedNodes.high)))
                return "visitedNodes: integer|Long expected";
        if (message.processedNodes != null && message.hasOwnProperty("processedNodes"))
            if (!$util.isInteger(message.processedNodes) && !(message.processedNodes && $util.isInteger(message.processedNodes.low) && $util.isInteger(message.processedNodes.high)))
                return "processedNodes: integer|Long expected";
        if (message.queuedNodes != null && message.hasOwnProperty("queuedNodes"))
            if (!$util.isInteger(message.queuedNodes) && !(message.queuedNodes && $util.isInteger(message.queuedNodes.low) && $util.isInteger(message.queuedNodes.high)))
                return "queuedNodes: integer|Long expected";
        if (message.activeWorkers != null && message.hasOwnProperty("activeWorkers"))
            if (!$util.isInteger(message.activeWorkers))
                return "activeWorkers: integer expected";
        if (message.workerTarget != null && message.hasOwnProperty("workerTarget"))
            if (!$util.isInteger(message.workerTarget))
                return "workerTarget: integer expected";
        if (message.etaSeconds != null && message.hasOwnProperty("etaSeconds")) {
            properties._etaSeconds = 1;
            if (!$util.isInteger(message.etaSeconds) && !(message.etaSeconds && $util.isInteger(message.etaSeconds.low) && $util.isInteger(message.etaSeconds.high)))
                return "etaSeconds: integer|Long expected";
        }
        if (message.deltaNewFiles != null && message.hasOwnProperty("deltaNewFiles"))
            if (!$util.isInteger(message.deltaNewFiles) && !(message.deltaNewFiles && $util.isInteger(message.deltaNewFiles.low) && $util.isInteger(message.deltaNewFiles.high)))
                return "deltaNewFiles: integer|Long expected";
        return null;
    };

    /**
     * Creates a CrawlStatusFrame message from a plain object. Also converts values to their respective internal types.
     * @function fromObject
     * @memberof CrawlStatusFrame
     * @static
     * @param {Object.<string,*>} object Plain object
     * @returns {CrawlStatusFrame} CrawlStatusFrame
     */
    CrawlStatusFrame.fromObject = function fromObject(object) {
        if (object instanceof $root.CrawlStatusFrame)
            return object;
        let message = new $root.CrawlStatusFrame();
        if (object.phase != null)
            message.phase = String(object.phase);
        if (object.progressPercent != null)
            message.progressPercent = Number(object.progressPercent);
        if (object.visitedNodes != null)
            if ($util.Long)
                (message.visitedNodes = $util.Long.fromValue(object.visitedNodes)).unsigned = true;
            else if (typeof object.visitedNodes === "string")
                message.visitedNodes = parseInt(object.visitedNodes, 10);
            else if (typeof object.visitedNodes === "number")
                message.visitedNodes = object.visitedNodes;
            else if (typeof object.visitedNodes === "object")
                message.visitedNodes = new $util.LongBits(object.visitedNodes.low >>> 0, object.visitedNodes.high >>> 0).toNumber(true);
        if (object.processedNodes != null)
            if ($util.Long)
                (message.processedNodes = $util.Long.fromValue(object.processedNodes)).unsigned = true;
            else if (typeof object.processedNodes === "string")
                message.processedNodes = parseInt(object.processedNodes, 10);
            else if (typeof object.processedNodes === "number")
                message.processedNodes = object.processedNodes;
            else if (typeof object.processedNodes === "object")
                message.processedNodes = new $util.LongBits(object.processedNodes.low >>> 0, object.processedNodes.high >>> 0).toNumber(true);
        if (object.queuedNodes != null)
            if ($util.Long)
                (message.queuedNodes = $util.Long.fromValue(object.queuedNodes)).unsigned = true;
            else if (typeof object.queuedNodes === "string")
                message.queuedNodes = parseInt(object.queuedNodes, 10);
            else if (typeof object.queuedNodes === "number")
                message.queuedNodes = object.queuedNodes;
            else if (typeof object.queuedNodes === "object")
                message.queuedNodes = new $util.LongBits(object.queuedNodes.low >>> 0, object.queuedNodes.high >>> 0).toNumber(true);
        if (object.activeWorkers != null)
            message.activeWorkers = object.activeWorkers >>> 0;
        if (object.workerTarget != null)
            message.workerTarget = object.workerTarget >>> 0;
        if (object.etaSeconds != null)
            if ($util.Long)
                (message.etaSeconds = $util.Long.fromValue(object.etaSeconds)).unsigned = true;
            else if (typeof object.etaSeconds === "string")
                message.etaSeconds = parseInt(object.etaSeconds, 10);
            else if (typeof object.etaSeconds === "number")
                message.etaSeconds = object.etaSeconds;
            else if (typeof object.etaSeconds === "object")
                message.etaSeconds = new $util.LongBits(object.etaSeconds.low >>> 0, object.etaSeconds.high >>> 0).toNumber(true);
        if (object.deltaNewFiles != null)
            if ($util.Long)
                (message.deltaNewFiles = $util.Long.fromValue(object.deltaNewFiles)).unsigned = true;
            else if (typeof object.deltaNewFiles === "string")
                message.deltaNewFiles = parseInt(object.deltaNewFiles, 10);
            else if (typeof object.deltaNewFiles === "number")
                message.deltaNewFiles = object.deltaNewFiles;
            else if (typeof object.deltaNewFiles === "object")
                message.deltaNewFiles = new $util.LongBits(object.deltaNewFiles.low >>> 0, object.deltaNewFiles.high >>> 0).toNumber(true);
        return message;
    };

    /**
     * Creates a plain object from a CrawlStatusFrame message. Also converts values to other types if specified.
     * @function toObject
     * @memberof CrawlStatusFrame
     * @static
     * @param {CrawlStatusFrame} message CrawlStatusFrame
     * @param {$protobuf.IConversionOptions} [options] Conversion options
     * @returns {Object.<string,*>} Plain object
     */
    CrawlStatusFrame.toObject = function toObject(message, options) {
        if (!options)
            options = {};
        let object = {};
        if (options.defaults) {
            object.phase = "";
            object.progressPercent = 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.visitedNodes = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.visitedNodes = options.longs === String ? "0" : 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.processedNodes = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.processedNodes = options.longs === String ? "0" : 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.queuedNodes = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.queuedNodes = options.longs === String ? "0" : 0;
            object.activeWorkers = 0;
            object.workerTarget = 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.deltaNewFiles = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.deltaNewFiles = options.longs === String ? "0" : 0;
        }
        if (message.phase != null && message.hasOwnProperty("phase"))
            object.phase = message.phase;
        if (message.progressPercent != null && message.hasOwnProperty("progressPercent"))
            object.progressPercent = options.json && !isFinite(message.progressPercent) ? String(message.progressPercent) : message.progressPercent;
        if (message.visitedNodes != null && message.hasOwnProperty("visitedNodes"))
            if (typeof message.visitedNodes === "number")
                object.visitedNodes = options.longs === String ? String(message.visitedNodes) : message.visitedNodes;
            else
                object.visitedNodes = options.longs === String ? $util.Long.prototype.toString.call(message.visitedNodes) : options.longs === Number ? new $util.LongBits(message.visitedNodes.low >>> 0, message.visitedNodes.high >>> 0).toNumber(true) : message.visitedNodes;
        if (message.processedNodes != null && message.hasOwnProperty("processedNodes"))
            if (typeof message.processedNodes === "number")
                object.processedNodes = options.longs === String ? String(message.processedNodes) : message.processedNodes;
            else
                object.processedNodes = options.longs === String ? $util.Long.prototype.toString.call(message.processedNodes) : options.longs === Number ? new $util.LongBits(message.processedNodes.low >>> 0, message.processedNodes.high >>> 0).toNumber(true) : message.processedNodes;
        if (message.queuedNodes != null && message.hasOwnProperty("queuedNodes"))
            if (typeof message.queuedNodes === "number")
                object.queuedNodes = options.longs === String ? String(message.queuedNodes) : message.queuedNodes;
            else
                object.queuedNodes = options.longs === String ? $util.Long.prototype.toString.call(message.queuedNodes) : options.longs === Number ? new $util.LongBits(message.queuedNodes.low >>> 0, message.queuedNodes.high >>> 0).toNumber(true) : message.queuedNodes;
        if (message.activeWorkers != null && message.hasOwnProperty("activeWorkers"))
            object.activeWorkers = message.activeWorkers;
        if (message.workerTarget != null && message.hasOwnProperty("workerTarget"))
            object.workerTarget = message.workerTarget;
        if (message.etaSeconds != null && message.hasOwnProperty("etaSeconds")) {
            if (typeof message.etaSeconds === "number")
                object.etaSeconds = options.longs === String ? String(message.etaSeconds) : message.etaSeconds;
            else
                object.etaSeconds = options.longs === String ? $util.Long.prototype.toString.call(message.etaSeconds) : options.longs === Number ? new $util.LongBits(message.etaSeconds.low >>> 0, message.etaSeconds.high >>> 0).toNumber(true) : message.etaSeconds;
            if (options.oneofs)
                object._etaSeconds = "etaSeconds";
        }
        if (message.deltaNewFiles != null && message.hasOwnProperty("deltaNewFiles"))
            if (typeof message.deltaNewFiles === "number")
                object.deltaNewFiles = options.longs === String ? String(message.deltaNewFiles) : message.deltaNewFiles;
            else
                object.deltaNewFiles = options.longs === String ? $util.Long.prototype.toString.call(message.deltaNewFiles) : options.longs === Number ? new $util.LongBits(message.deltaNewFiles.low >>> 0, message.deltaNewFiles.high >>> 0).toNumber(true) : message.deltaNewFiles;
        return object;
    };

    /**
     * Converts this CrawlStatusFrame to JSON.
     * @function toJSON
     * @memberof CrawlStatusFrame
     * @instance
     * @returns {Object.<string,*>} JSON object
     */
    CrawlStatusFrame.prototype.toJSON = function toJSON() {
        return this.constructor.toObject(this, $protobuf.util.toJSONOptions);
    };

    /**
     * Gets the default type url for CrawlStatusFrame
     * @function getTypeUrl
     * @memberof CrawlStatusFrame
     * @static
     * @param {string} [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns {string} The default type url
     */
    CrawlStatusFrame.getTypeUrl = function getTypeUrl(typeUrlPrefix) {
        if (typeUrlPrefix === undefined) {
            typeUrlPrefix = "type.googleapis.com";
        }
        return typeUrlPrefix + "/CrawlStatusFrame";
    };

    return CrawlStatusFrame;
})();

export const BatchProgressFrame = $root.BatchProgressFrame = (() => {

    /**
     * Properties of a BatchProgressFrame.
     * @exports IBatchProgressFrame
     * @interface IBatchProgressFrame
     * @property {number|Long|null} [completed] BatchProgressFrame completed
     * @property {number|Long|null} [failed] BatchProgressFrame failed
     * @property {number|Long|null} [total] BatchProgressFrame total
     * @property {string|null} [currentFile] BatchProgressFrame currentFile
     * @property {number|Long|null} [downloadedBytes] BatchProgressFrame downloadedBytes
     * @property {number|null} [activeCircuits] BatchProgressFrame activeCircuits
     */

    /**
     * Constructs a new BatchProgressFrame.
     * @exports BatchProgressFrame
     * @classdesc Represents a BatchProgressFrame.
     * @implements IBatchProgressFrame
     * @constructor
     * @param {IBatchProgressFrame=} [properties] Properties to set
     */
    function BatchProgressFrame(properties) {
        if (properties)
            for (let keys = Object.keys(properties), i = 0; i < keys.length; ++i)
                if (properties[keys[i]] != null)
                    this[keys[i]] = properties[keys[i]];
    }

    /**
     * BatchProgressFrame completed.
     * @member {number|Long} completed
     * @memberof BatchProgressFrame
     * @instance
     */
    BatchProgressFrame.prototype.completed = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * BatchProgressFrame failed.
     * @member {number|Long} failed
     * @memberof BatchProgressFrame
     * @instance
     */
    BatchProgressFrame.prototype.failed = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * BatchProgressFrame total.
     * @member {number|Long} total
     * @memberof BatchProgressFrame
     * @instance
     */
    BatchProgressFrame.prototype.total = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * BatchProgressFrame currentFile.
     * @member {string} currentFile
     * @memberof BatchProgressFrame
     * @instance
     */
    BatchProgressFrame.prototype.currentFile = "";

    /**
     * BatchProgressFrame downloadedBytes.
     * @member {number|Long} downloadedBytes
     * @memberof BatchProgressFrame
     * @instance
     */
    BatchProgressFrame.prototype.downloadedBytes = $util.Long ? $util.Long.fromBits(0,0,true) : 0;

    /**
     * BatchProgressFrame activeCircuits.
     * @member {number|null|undefined} activeCircuits
     * @memberof BatchProgressFrame
     * @instance
     */
    BatchProgressFrame.prototype.activeCircuits = null;

    // OneOf field names bound to virtual getters and setters
    let $oneOfFields;

    // Virtual OneOf for proto3 optional field
    Object.defineProperty(BatchProgressFrame.prototype, "_activeCircuits", {
        get: $util.oneOfGetter($oneOfFields = ["activeCircuits"]),
        set: $util.oneOfSetter($oneOfFields)
    });

    /**
     * Creates a new BatchProgressFrame instance using the specified properties.
     * @function create
     * @memberof BatchProgressFrame
     * @static
     * @param {IBatchProgressFrame=} [properties] Properties to set
     * @returns {BatchProgressFrame} BatchProgressFrame instance
     */
    BatchProgressFrame.create = function create(properties) {
        return new BatchProgressFrame(properties);
    };

    /**
     * Encodes the specified BatchProgressFrame message. Does not implicitly {@link BatchProgressFrame.verify|verify} messages.
     * @function encode
     * @memberof BatchProgressFrame
     * @static
     * @param {IBatchProgressFrame} message BatchProgressFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    BatchProgressFrame.encode = function encode(message, writer) {
        if (!writer)
            writer = $Writer.create();
        if (message.completed != null && Object.hasOwnProperty.call(message, "completed"))
            writer.uint32(/* id 1, wireType 0 =*/8).uint64(message.completed);
        if (message.failed != null && Object.hasOwnProperty.call(message, "failed"))
            writer.uint32(/* id 2, wireType 0 =*/16).uint64(message.failed);
        if (message.total != null && Object.hasOwnProperty.call(message, "total"))
            writer.uint32(/* id 3, wireType 0 =*/24).uint64(message.total);
        if (message.currentFile != null && Object.hasOwnProperty.call(message, "currentFile"))
            writer.uint32(/* id 4, wireType 2 =*/34).string(message.currentFile);
        if (message.downloadedBytes != null && Object.hasOwnProperty.call(message, "downloadedBytes"))
            writer.uint32(/* id 5, wireType 0 =*/40).uint64(message.downloadedBytes);
        if (message.activeCircuits != null && Object.hasOwnProperty.call(message, "activeCircuits"))
            writer.uint32(/* id 6, wireType 0 =*/48).uint32(message.activeCircuits);
        return writer;
    };

    /**
     * Encodes the specified BatchProgressFrame message, length delimited. Does not implicitly {@link BatchProgressFrame.verify|verify} messages.
     * @function encodeDelimited
     * @memberof BatchProgressFrame
     * @static
     * @param {IBatchProgressFrame} message BatchProgressFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    BatchProgressFrame.encodeDelimited = function encodeDelimited(message, writer) {
        return this.encode(message, writer).ldelim();
    };

    /**
     * Decodes a BatchProgressFrame message from the specified reader or buffer.
     * @function decode
     * @memberof BatchProgressFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @param {number} [length] Message length if known beforehand
     * @returns {BatchProgressFrame} BatchProgressFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    BatchProgressFrame.decode = function decode(reader, length, error) {
        if (!(reader instanceof $Reader))
            reader = $Reader.create(reader);
        let end = length === undefined ? reader.len : reader.pos + length, message = new $root.BatchProgressFrame();
        while (reader.pos < end) {
            let tag = reader.uint32();
            if (tag === error)
                break;
            switch (tag >>> 3) {
            case 1: {
                    message.completed = reader.uint64();
                    break;
                }
            case 2: {
                    message.failed = reader.uint64();
                    break;
                }
            case 3: {
                    message.total = reader.uint64();
                    break;
                }
            case 4: {
                    message.currentFile = reader.string();
                    break;
                }
            case 5: {
                    message.downloadedBytes = reader.uint64();
                    break;
                }
            case 6: {
                    message.activeCircuits = reader.uint32();
                    break;
                }
            default:
                reader.skipType(tag & 7);
                break;
            }
        }
        return message;
    };

    /**
     * Decodes a BatchProgressFrame message from the specified reader or buffer, length delimited.
     * @function decodeDelimited
     * @memberof BatchProgressFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @returns {BatchProgressFrame} BatchProgressFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    BatchProgressFrame.decodeDelimited = function decodeDelimited(reader) {
        if (!(reader instanceof $Reader))
            reader = new $Reader(reader);
        return this.decode(reader, reader.uint32());
    };

    /**
     * Verifies a BatchProgressFrame message.
     * @function verify
     * @memberof BatchProgressFrame
     * @static
     * @param {Object.<string,*>} message Plain object to verify
     * @returns {string|null} `null` if valid, otherwise the reason why it is not
     */
    BatchProgressFrame.verify = function verify(message) {
        if (typeof message !== "object" || message === null)
            return "object expected";
        let properties = {};
        if (message.completed != null && message.hasOwnProperty("completed"))
            if (!$util.isInteger(message.completed) && !(message.completed && $util.isInteger(message.completed.low) && $util.isInteger(message.completed.high)))
                return "completed: integer|Long expected";
        if (message.failed != null && message.hasOwnProperty("failed"))
            if (!$util.isInteger(message.failed) && !(message.failed && $util.isInteger(message.failed.low) && $util.isInteger(message.failed.high)))
                return "failed: integer|Long expected";
        if (message.total != null && message.hasOwnProperty("total"))
            if (!$util.isInteger(message.total) && !(message.total && $util.isInteger(message.total.low) && $util.isInteger(message.total.high)))
                return "total: integer|Long expected";
        if (message.currentFile != null && message.hasOwnProperty("currentFile"))
            if (!$util.isString(message.currentFile))
                return "currentFile: string expected";
        if (message.downloadedBytes != null && message.hasOwnProperty("downloadedBytes"))
            if (!$util.isInteger(message.downloadedBytes) && !(message.downloadedBytes && $util.isInteger(message.downloadedBytes.low) && $util.isInteger(message.downloadedBytes.high)))
                return "downloadedBytes: integer|Long expected";
        if (message.activeCircuits != null && message.hasOwnProperty("activeCircuits")) {
            properties._activeCircuits = 1;
            if (!$util.isInteger(message.activeCircuits))
                return "activeCircuits: integer expected";
        }
        return null;
    };

    /**
     * Creates a BatchProgressFrame message from a plain object. Also converts values to their respective internal types.
     * @function fromObject
     * @memberof BatchProgressFrame
     * @static
     * @param {Object.<string,*>} object Plain object
     * @returns {BatchProgressFrame} BatchProgressFrame
     */
    BatchProgressFrame.fromObject = function fromObject(object) {
        if (object instanceof $root.BatchProgressFrame)
            return object;
        let message = new $root.BatchProgressFrame();
        if (object.completed != null)
            if ($util.Long)
                (message.completed = $util.Long.fromValue(object.completed)).unsigned = true;
            else if (typeof object.completed === "string")
                message.completed = parseInt(object.completed, 10);
            else if (typeof object.completed === "number")
                message.completed = object.completed;
            else if (typeof object.completed === "object")
                message.completed = new $util.LongBits(object.completed.low >>> 0, object.completed.high >>> 0).toNumber(true);
        if (object.failed != null)
            if ($util.Long)
                (message.failed = $util.Long.fromValue(object.failed)).unsigned = true;
            else if (typeof object.failed === "string")
                message.failed = parseInt(object.failed, 10);
            else if (typeof object.failed === "number")
                message.failed = object.failed;
            else if (typeof object.failed === "object")
                message.failed = new $util.LongBits(object.failed.low >>> 0, object.failed.high >>> 0).toNumber(true);
        if (object.total != null)
            if ($util.Long)
                (message.total = $util.Long.fromValue(object.total)).unsigned = true;
            else if (typeof object.total === "string")
                message.total = parseInt(object.total, 10);
            else if (typeof object.total === "number")
                message.total = object.total;
            else if (typeof object.total === "object")
                message.total = new $util.LongBits(object.total.low >>> 0, object.total.high >>> 0).toNumber(true);
        if (object.currentFile != null)
            message.currentFile = String(object.currentFile);
        if (object.downloadedBytes != null)
            if ($util.Long)
                (message.downloadedBytes = $util.Long.fromValue(object.downloadedBytes)).unsigned = true;
            else if (typeof object.downloadedBytes === "string")
                message.downloadedBytes = parseInt(object.downloadedBytes, 10);
            else if (typeof object.downloadedBytes === "number")
                message.downloadedBytes = object.downloadedBytes;
            else if (typeof object.downloadedBytes === "object")
                message.downloadedBytes = new $util.LongBits(object.downloadedBytes.low >>> 0, object.downloadedBytes.high >>> 0).toNumber(true);
        if (object.activeCircuits != null)
            message.activeCircuits = object.activeCircuits >>> 0;
        return message;
    };

    /**
     * Creates a plain object from a BatchProgressFrame message. Also converts values to other types if specified.
     * @function toObject
     * @memberof BatchProgressFrame
     * @static
     * @param {BatchProgressFrame} message BatchProgressFrame
     * @param {$protobuf.IConversionOptions} [options] Conversion options
     * @returns {Object.<string,*>} Plain object
     */
    BatchProgressFrame.toObject = function toObject(message, options) {
        if (!options)
            options = {};
        let object = {};
        if (options.defaults) {
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.completed = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.completed = options.longs === String ? "0" : 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.failed = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.failed = options.longs === String ? "0" : 0;
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.total = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.total = options.longs === String ? "0" : 0;
            object.currentFile = "";
            if ($util.Long) {
                let long = new $util.Long(0, 0, true);
                object.downloadedBytes = options.longs === String ? long.toString() : options.longs === Number ? long.toNumber() : long;
            } else
                object.downloadedBytes = options.longs === String ? "0" : 0;
        }
        if (message.completed != null && message.hasOwnProperty("completed"))
            if (typeof message.completed === "number")
                object.completed = options.longs === String ? String(message.completed) : message.completed;
            else
                object.completed = options.longs === String ? $util.Long.prototype.toString.call(message.completed) : options.longs === Number ? new $util.LongBits(message.completed.low >>> 0, message.completed.high >>> 0).toNumber(true) : message.completed;
        if (message.failed != null && message.hasOwnProperty("failed"))
            if (typeof message.failed === "number")
                object.failed = options.longs === String ? String(message.failed) : message.failed;
            else
                object.failed = options.longs === String ? $util.Long.prototype.toString.call(message.failed) : options.longs === Number ? new $util.LongBits(message.failed.low >>> 0, message.failed.high >>> 0).toNumber(true) : message.failed;
        if (message.total != null && message.hasOwnProperty("total"))
            if (typeof message.total === "number")
                object.total = options.longs === String ? String(message.total) : message.total;
            else
                object.total = options.longs === String ? $util.Long.prototype.toString.call(message.total) : options.longs === Number ? new $util.LongBits(message.total.low >>> 0, message.total.high >>> 0).toNumber(true) : message.total;
        if (message.currentFile != null && message.hasOwnProperty("currentFile"))
            object.currentFile = message.currentFile;
        if (message.downloadedBytes != null && message.hasOwnProperty("downloadedBytes"))
            if (typeof message.downloadedBytes === "number")
                object.downloadedBytes = options.longs === String ? String(message.downloadedBytes) : message.downloadedBytes;
            else
                object.downloadedBytes = options.longs === String ? $util.Long.prototype.toString.call(message.downloadedBytes) : options.longs === Number ? new $util.LongBits(message.downloadedBytes.low >>> 0, message.downloadedBytes.high >>> 0).toNumber(true) : message.downloadedBytes;
        if (message.activeCircuits != null && message.hasOwnProperty("activeCircuits")) {
            object.activeCircuits = message.activeCircuits;
            if (options.oneofs)
                object._activeCircuits = "activeCircuits";
        }
        return object;
    };

    /**
     * Converts this BatchProgressFrame to JSON.
     * @function toJSON
     * @memberof BatchProgressFrame
     * @instance
     * @returns {Object.<string,*>} JSON object
     */
    BatchProgressFrame.prototype.toJSON = function toJSON() {
        return this.constructor.toObject(this, $protobuf.util.toJSONOptions);
    };

    /**
     * Gets the default type url for BatchProgressFrame
     * @function getTypeUrl
     * @memberof BatchProgressFrame
     * @static
     * @param {string} [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns {string} The default type url
     */
    BatchProgressFrame.getTypeUrl = function getTypeUrl(typeUrlPrefix) {
        if (typeUrlPrefix === undefined) {
            typeUrlPrefix = "type.googleapis.com";
        }
        return typeUrlPrefix + "/BatchProgressFrame";
    };

    return BatchProgressFrame;
})();

export const DownloadStatusFrame = $root.DownloadStatusFrame = (() => {

    /**
     * Properties of a DownloadStatusFrame.
     * @exports IDownloadStatusFrame
     * @interface IDownloadStatusFrame
     * @property {string|null} [phase] DownloadStatusFrame phase
     * @property {string|null} [message] DownloadStatusFrame message
     * @property {number|null} [downloadTimeSecs] DownloadStatusFrame downloadTimeSecs
     * @property {number|null} [percent] DownloadStatusFrame percent
     */

    /**
     * Constructs a new DownloadStatusFrame.
     * @exports DownloadStatusFrame
     * @classdesc Represents a DownloadStatusFrame.
     * @implements IDownloadStatusFrame
     * @constructor
     * @param {IDownloadStatusFrame=} [properties] Properties to set
     */
    function DownloadStatusFrame(properties) {
        if (properties)
            for (let keys = Object.keys(properties), i = 0; i < keys.length; ++i)
                if (properties[keys[i]] != null)
                    this[keys[i]] = properties[keys[i]];
    }

    /**
     * DownloadStatusFrame phase.
     * @member {string} phase
     * @memberof DownloadStatusFrame
     * @instance
     */
    DownloadStatusFrame.prototype.phase = "";

    /**
     * DownloadStatusFrame message.
     * @member {string} message
     * @memberof DownloadStatusFrame
     * @instance
     */
    DownloadStatusFrame.prototype.message = "";

    /**
     * DownloadStatusFrame downloadTimeSecs.
     * @member {number|null|undefined} downloadTimeSecs
     * @memberof DownloadStatusFrame
     * @instance
     */
    DownloadStatusFrame.prototype.downloadTimeSecs = null;

    /**
     * DownloadStatusFrame percent.
     * @member {number|null|undefined} percent
     * @memberof DownloadStatusFrame
     * @instance
     */
    DownloadStatusFrame.prototype.percent = null;

    // OneOf field names bound to virtual getters and setters
    let $oneOfFields;

    // Virtual OneOf for proto3 optional field
    Object.defineProperty(DownloadStatusFrame.prototype, "_downloadTimeSecs", {
        get: $util.oneOfGetter($oneOfFields = ["downloadTimeSecs"]),
        set: $util.oneOfSetter($oneOfFields)
    });

    // Virtual OneOf for proto3 optional field
    Object.defineProperty(DownloadStatusFrame.prototype, "_percent", {
        get: $util.oneOfGetter($oneOfFields = ["percent"]),
        set: $util.oneOfSetter($oneOfFields)
    });

    /**
     * Creates a new DownloadStatusFrame instance using the specified properties.
     * @function create
     * @memberof DownloadStatusFrame
     * @static
     * @param {IDownloadStatusFrame=} [properties] Properties to set
     * @returns {DownloadStatusFrame} DownloadStatusFrame instance
     */
    DownloadStatusFrame.create = function create(properties) {
        return new DownloadStatusFrame(properties);
    };

    /**
     * Encodes the specified DownloadStatusFrame message. Does not implicitly {@link DownloadStatusFrame.verify|verify} messages.
     * @function encode
     * @memberof DownloadStatusFrame
     * @static
     * @param {IDownloadStatusFrame} message DownloadStatusFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    DownloadStatusFrame.encode = function encode(message, writer) {
        if (!writer)
            writer = $Writer.create();
        if (message.phase != null && Object.hasOwnProperty.call(message, "phase"))
            writer.uint32(/* id 1, wireType 2 =*/10).string(message.phase);
        if (message.message != null && Object.hasOwnProperty.call(message, "message"))
            writer.uint32(/* id 2, wireType 2 =*/18).string(message.message);
        if (message.downloadTimeSecs != null && Object.hasOwnProperty.call(message, "downloadTimeSecs"))
            writer.uint32(/* id 3, wireType 1 =*/25).double(message.downloadTimeSecs);
        if (message.percent != null && Object.hasOwnProperty.call(message, "percent"))
            writer.uint32(/* id 4, wireType 1 =*/33).double(message.percent);
        return writer;
    };

    /**
     * Encodes the specified DownloadStatusFrame message, length delimited. Does not implicitly {@link DownloadStatusFrame.verify|verify} messages.
     * @function encodeDelimited
     * @memberof DownloadStatusFrame
     * @static
     * @param {IDownloadStatusFrame} message DownloadStatusFrame message or plain object to encode
     * @param {$protobuf.Writer} [writer] Writer to encode to
     * @returns {$protobuf.Writer} Writer
     */
    DownloadStatusFrame.encodeDelimited = function encodeDelimited(message, writer) {
        return this.encode(message, writer).ldelim();
    };

    /**
     * Decodes a DownloadStatusFrame message from the specified reader or buffer.
     * @function decode
     * @memberof DownloadStatusFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @param {number} [length] Message length if known beforehand
     * @returns {DownloadStatusFrame} DownloadStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    DownloadStatusFrame.decode = function decode(reader, length, error) {
        if (!(reader instanceof $Reader))
            reader = $Reader.create(reader);
        let end = length === undefined ? reader.len : reader.pos + length, message = new $root.DownloadStatusFrame();
        while (reader.pos < end) {
            let tag = reader.uint32();
            if (tag === error)
                break;
            switch (tag >>> 3) {
            case 1: {
                    message.phase = reader.string();
                    break;
                }
            case 2: {
                    message.message = reader.string();
                    break;
                }
            case 3: {
                    message.downloadTimeSecs = reader.double();
                    break;
                }
            case 4: {
                    message.percent = reader.double();
                    break;
                }
            default:
                reader.skipType(tag & 7);
                break;
            }
        }
        return message;
    };

    /**
     * Decodes a DownloadStatusFrame message from the specified reader or buffer, length delimited.
     * @function decodeDelimited
     * @memberof DownloadStatusFrame
     * @static
     * @param {$protobuf.Reader|Uint8Array} reader Reader or buffer to decode from
     * @returns {DownloadStatusFrame} DownloadStatusFrame
     * @throws {Error} If the payload is not a reader or valid buffer
     * @throws {$protobuf.util.ProtocolError} If required fields are missing
     */
    DownloadStatusFrame.decodeDelimited = function decodeDelimited(reader) {
        if (!(reader instanceof $Reader))
            reader = new $Reader(reader);
        return this.decode(reader, reader.uint32());
    };

    /**
     * Verifies a DownloadStatusFrame message.
     * @function verify
     * @memberof DownloadStatusFrame
     * @static
     * @param {Object.<string,*>} message Plain object to verify
     * @returns {string|null} `null` if valid, otherwise the reason why it is not
     */
    DownloadStatusFrame.verify = function verify(message) {
        if (typeof message !== "object" || message === null)
            return "object expected";
        let properties = {};
        if (message.phase != null && message.hasOwnProperty("phase"))
            if (!$util.isString(message.phase))
                return "phase: string expected";
        if (message.message != null && message.hasOwnProperty("message"))
            if (!$util.isString(message.message))
                return "message: string expected";
        if (message.downloadTimeSecs != null && message.hasOwnProperty("downloadTimeSecs")) {
            properties._downloadTimeSecs = 1;
            if (typeof message.downloadTimeSecs !== "number")
                return "downloadTimeSecs: number expected";
        }
        if (message.percent != null && message.hasOwnProperty("percent")) {
            properties._percent = 1;
            if (typeof message.percent !== "number")
                return "percent: number expected";
        }
        return null;
    };

    /**
     * Creates a DownloadStatusFrame message from a plain object. Also converts values to their respective internal types.
     * @function fromObject
     * @memberof DownloadStatusFrame
     * @static
     * @param {Object.<string,*>} object Plain object
     * @returns {DownloadStatusFrame} DownloadStatusFrame
     */
    DownloadStatusFrame.fromObject = function fromObject(object) {
        if (object instanceof $root.DownloadStatusFrame)
            return object;
        let message = new $root.DownloadStatusFrame();
        if (object.phase != null)
            message.phase = String(object.phase);
        if (object.message != null)
            message.message = String(object.message);
        if (object.downloadTimeSecs != null)
            message.downloadTimeSecs = Number(object.downloadTimeSecs);
        if (object.percent != null)
            message.percent = Number(object.percent);
        return message;
    };

    /**
     * Creates a plain object from a DownloadStatusFrame message. Also converts values to other types if specified.
     * @function toObject
     * @memberof DownloadStatusFrame
     * @static
     * @param {DownloadStatusFrame} message DownloadStatusFrame
     * @param {$protobuf.IConversionOptions} [options] Conversion options
     * @returns {Object.<string,*>} Plain object
     */
    DownloadStatusFrame.toObject = function toObject(message, options) {
        if (!options)
            options = {};
        let object = {};
        if (options.defaults) {
            object.phase = "";
            object.message = "";
        }
        if (message.phase != null && message.hasOwnProperty("phase"))
            object.phase = message.phase;
        if (message.message != null && message.hasOwnProperty("message"))
            object.message = message.message;
        if (message.downloadTimeSecs != null && message.hasOwnProperty("downloadTimeSecs")) {
            object.downloadTimeSecs = options.json && !isFinite(message.downloadTimeSecs) ? String(message.downloadTimeSecs) : message.downloadTimeSecs;
            if (options.oneofs)
                object._downloadTimeSecs = "downloadTimeSecs";
        }
        if (message.percent != null && message.hasOwnProperty("percent")) {
            object.percent = options.json && !isFinite(message.percent) ? String(message.percent) : message.percent;
            if (options.oneofs)
                object._percent = "percent";
        }
        return object;
    };

    /**
     * Converts this DownloadStatusFrame to JSON.
     * @function toJSON
     * @memberof DownloadStatusFrame
     * @instance
     * @returns {Object.<string,*>} JSON object
     */
    DownloadStatusFrame.prototype.toJSON = function toJSON() {
        return this.constructor.toObject(this, $protobuf.util.toJSONOptions);
    };

    /**
     * Gets the default type url for DownloadStatusFrame
     * @function getTypeUrl
     * @memberof DownloadStatusFrame
     * @static
     * @param {string} [typeUrlPrefix] your custom typeUrlPrefix(default "type.googleapis.com")
     * @returns {string} The default type url
     */
    DownloadStatusFrame.getTypeUrl = function getTypeUrl(typeUrlPrefix) {
        if (typeUrlPrefix === undefined) {
            typeUrlPrefix = "type.googleapis.com";
        }
        return typeUrlPrefix + "/DownloadStatusFrame";
    };

    return DownloadStatusFrame;
})();

export { $root as default };
