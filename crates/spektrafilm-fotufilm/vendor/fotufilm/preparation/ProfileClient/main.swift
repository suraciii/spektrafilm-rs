import Foundation
import FotufilmEditModel
import FotufilmCore
import FotufilmHalide

struct AlignmentEnvelope: Decodable {
    struct Settings: Decodable {
        var stage: String?; var seed: UInt64?; var frameIndex: UInt64?
        var textureStages: Int32?
        var sceneIlluminantKelvin: Float?
        var sceneIlluminantChromaticity: [Float]?
        var sceneIlluminantSpectrum: [Float]?
        var layeredTransport: LayeredTransport?
        var halationModel: HalationModel?
        var scalars: [String: Float]?
        var flags: [String: Bool]?
        var arrays: [String: [Float]]?
        var whiteBalance: WhiteBalance?
        var grade: ColorGrade?
        var printer: PrinterProfile?
        var receiverBands: ReceiverBands?
        var diffusionFilter: DiffusionFilter?
        var grainModel: GrainModel?
        var adjacencyModel: AdjacencyModel?
        var enlarger: Enlarger?
        var digitalReference: DigitalReferenceStyle?
        var paper: String?
        var negativeViewing: String?
        var gradeSpace: String?
        struct Filters: Decodable { var filters: [LensFilter]; var compensation: LensFilterCompensation?; var lensFrontReflectance: Float? }
        struct Format: Decodable { var name: String; var frameHeightMM: Float; var frameAspectRatio: Float; var thicknessMM: Float; var refractiveIndex: Float }
        struct Look: Decodable { var layers: [Float]; var colour: Float; var size: Float; var softness: Float }
        struct Finish: Decodable { var gains: [Float]; var highlights: Float; var shadows: Float; var saturation: Float; var vibrance: Float }
        var lensFilters: Filters?
        var format: Format?
        var filmGrain: Look?
        var printFinish: Finish?
        var vectors: [String: [Float]]?
        struct Scan: Decodable { var border: [Float]; var gains: [Float]? }
        struct Edge: Decodable { var left: Int; var right: Int; var top: Int; var bottom: Int; var carrier: Int?; var fNumber: Float?; var sceneHighlightStops: Float?; var sceneChannelMedians: [Float]?; var sceneToneStops: [Float]? }
        var scanReading: Scan?
        var unexposedEdge: Edge?
    }
    var alignment: Settings?
}
struct AlignmentFailure: Error, CustomStringConvertible {
    var description: String
    init(_ text: String) { description = text }
}

func applyNativeOptions(_ settings: AlignmentEnvelope.Settings?, to options: inout FotufilmEngine.Options) throws {
    for (key, value) in settings?.scalars ?? [:] {
        guard value.isFinite else { throw AlignmentFailure("Non-finite native scalar") }
        switch key {
        case "exposureEV": options.exposureEV = value
        case "sceneHeadroom": options.sceneHeadroom = value
        case "hdrRange": options.hdrRange = value
        case "hdrRollOff": options.hdrRollOff = value
        case "highlights": options.highlights = value
        case "shadows": options.shadows = value
        case "saturation": options.saturation = value
        case "vibrance": options.vibrance = value
        case "grainScale": options.grainScale = value
        case "halationScale": options.halationScale = value
        case "halationSourceColour": options.halationSourceColour = value
        case "antiHalationScale": options.antiHalationScale = value
        case "baseThicknessScale": options.baseThicknessScale = value
        case "pressurePlateReflectance": options.pressurePlateReflectance = value
        case "flareScale": options.flareScale = value
        case "cameraPreflash": options.cameraPreflash = value
        case "couplerScale": options.couplerScale = value
        case "couplerRangeScale": options.couplerRangeScale = value
        case "couplerSelfScale": options.couplerSelfScale = value
        case "expiredYears": options.expiredYears = value
        case "developmentEV": options.developmentEV = value
        case "bleachBypass": options.bleachBypass = value
        case "printCorrection": options.printCorrection = value
        case "printerPreflash": options.printerPreflash = value
        case "frameCoverage": options.frameCoverage = value
        case "screenGrade": options.screenGrade = value
        case "screenExposureEV": options.screenExposureEV = value
        case "labScanDodging": options.labScanDodging = value
        case "labScanLook": options.labScanLook = value
        case "grainMottleShare": options.grainMottleShare = value
        case "grainMottleSizeRatio": options.grainMottleSizeRatio = value
        case "halationReturnRatio": options.halationReturnRatio = value
        case "halationHazeMM": options.halationHazeMM = value
        case "chromaticFringeAmount": options.chromaticFringeAmount = value
        case "chromaticFringeRadiusMM": options.chromaticFringeRadiusMM = value
        case "shutterSeconds": options.shutterSeconds = value
        case "printViewingKelvin": options.printViewingKelvin = value
        case "sceneHighlightStops": options.sceneHighlightStops = value
        case "sceneIlluminantKelvin": options.sceneIlluminantKelvin = value
        case "focalLengthMM": options.focalLengthMM = value
        default: throw AlignmentFailure("Unknown native scalar: \(key)")
        }
    }
    for (key, value) in settings?.flags ?? [:] {
        switch key {
        case "localTone": options.localTone = value
        case "useEstimatedHalationProfile": options.useEstimatedHalationProfile = value
        default: throw AlignmentFailure("Unknown native flag: \(key)")
        }
    }
    for (key, value) in settings?.arrays ?? [:] {
        switch key {
        case "halationReturnGain": options.halationReturnGain = value
        case "couplerGapReachScales": options.couplerGapReachScales = value
        case "sceneIlluminantSpectrum": options.sceneIlluminantSpectrum = value
        default: throw AlignmentFailure("Unknown native array: \(key)")
        }
    }
    if let value = settings?.whiteBalance { options.whiteBalance = value }
    if let value = settings?.grade { options.grade = value }
    if let value = settings?.printer { options.printer = value }
    if let value = settings?.receiverBands { options.receiverBands = value }
    if let value = settings?.diffusionFilter { options.diffusionFilter = value }
    if let value = settings?.grainModel { options.grainModel = value }
    if let value = settings?.adjacencyModel { options.adjacencyModel = value }
    if let value = settings?.enlarger { options.enlarger = value }
    if let value = settings?.digitalReference { options.digitalReference = value }
    if let value = settings?.paper {
        guard let paper = PrintPaper.preset(id: value) else { throw AlignmentFailure("Unknown native paper") }
        options.paper = paper
    }
    if let value = settings?.negativeViewing {
        guard let look = NegativeViewing(rawValue: value) else { throw AlignmentFailure("Unknown negative viewing") }
        options.negativeViewing = look
    }
    if let value = settings?.gradeSpace {
        guard let space = ColorGrade.Space(rawValue: value) else { throw AlignmentFailure("Unknown grade space") }
        options.gradeSpace = space
    }
    if let stack = settings?.lensFilters {
        options.lensFilters = LensFilterStack(stack.filters, compensation: stack.compensation ?? .throughTheLens,
                                             lensFrontReflectance: stack.lensFrontReflectance ?? 0.005)
    }
    if let format = settings?.format {
        options.format = FilmFormat(name: format.name, frameHeightMM: format.frameHeightMM,
            frameAspectRatio: format.frameAspectRatio, base: FilmBase(thicknessMM: format.thicknessMM, refractiveIndex: format.refractiveIndex))
    }
    if let look = settings?.filmGrain {
        guard look.layers.count == 3 else { throw AlignmentFailure("Film grain requires three layer amounts") }
        options.filmGrain = FilmGrain.Look(layers: SIMD3(look.layers[0], look.layers[1], look.layers[2]), colour: look.colour, size: look.size, softness: look.softness)
    }
    if let finish = settings?.printFinish {
        guard finish.gains.count == 3 else { throw AlignmentFailure("Print finish requires three gains") }
        options.printFinish = PrintFinish(gains: SIMD3(finish.gains[0], finish.gains[1], finish.gains[2]), highlights: finish.highlights, shadows: finish.shadows, saturation: finish.saturation, vibrance: finish.vibrance)
    }
    for (key, vector) in settings?.vectors ?? [:] {
        guard vector.count == 3 else { throw AlignmentFailure("Native vector requires three components") }
        let value = SIMD3(vector[0], vector[1], vector[2])
        switch key {
        case "sceneChannelMedians": options.sceneChannelMedians = value
        case "sceneToneStops": options.sceneToneStops = value
        case "screenCMY": options.screenCMY = value
        default: throw AlignmentFailure("Unknown native vector: \(key)")
        }
    }
}


func dynamicChoices(_ menu: EditorDynamicMenu, stock: FilmStock) -> [EditorMenuChoice] {
    switch menu {
    case .gauges: return FilmFormat.presets.map { .init(nil, $0.format.name, id: $0.id) }
    case .papers: return PrintPaper.choices(for: stock).map { .init(nil, $0.name, id: $0.id) }
    case .meterings: return LensFilterCompensation.allCases.map { .init(nil, $0.label, id: $0.rawValue) }
    case .negativeViewings: return NegativeViewing.allCases.enumerated().map { .init(Double($0.offset), $0.element.name, id: $0.element.rawValue) }
    case .stages: return PipelineStage.allCases.map { .init(Double($0.ordinal), $0.name, id: $0.rawValue) }
    case .textureStages: return [.init(Double(TextureStages.all.rawValue), "All", id: "all"), .init(0, "None", id: "none")] + TextureStages.ordered.map { .init(Double($0.value.rawValue), $0.name, id: $0.id) }
    case .lensFilters: return (EditorLensFilters.absorbing + EditorLensFilters.diffusion).map { .init(nil, $0.name, detail: $0.detail, id: $0.id) }
    case .diffusionFamilies: return DiffusionFilter.Family.allCases.enumerated().map { .init(Double($0.offset), $0.element.label, id: $0.element.rawValue) }
    case .diffusionGrades: return DiffusionFilter.Grade.allCases.enumerated().map { .init(Double($0.offset), $0.element.rawValue, id: $0.element.rawValue) }
    case .shutterTimes: return [.init(nil, "Instantaneous", id: "off")] + EditorControlCatalogue.shutterTimes(for: stock).map { .init($0, EditorControlCatalogue.shutterName($0), id: String(Int($0))) }
    case .viewingLights: return EditorControlCatalogue.viewingLights(for: .default(for: stock))
    case .colourSpaces: return EditorControlCatalogue.pluginWorkingSpaces
    case .pushConditions: return stock.supportedDevelopmentStops.map { .init(Double($0), String($0), id: String($0)) }
    case .stocks, .lensProfiles: return []
    }
}

func alignmentCatalogue(_ data: Data) throws -> Data {
    struct Request: Decodable { let stock: FilmStockDefinition }
    let definition = try JSONDecoder().decode(Request.self, from: data).stock
    let stock = try definition.validated().stock
    let manifestData = try JSONEncoder().encode(ControlsManifest.current)
    let manifest = try JSONSerialization.jsonObject(with: manifestData)
    let applicable = Dictionary(uniqueKeysWithValues: EditorControlCatalogue.all.map {
        ($0.field.rawValue, $0.availability.admits(stock: stock))
    })
    let surfaces = Dictionary(uniqueKeysWithValues: EditorSurface.allCases.map { surface in
        (surface.rawValue, EditorControlCatalogue.controls(for: stock, on: surface).map { $0.field.rawValue })
    })
    let media = PrintPaper.choices(for: stock).map { paper -> [String: Any] in
        ["id": paper.id, "available": EditorControlCatalogue.all.filter {
            EditorControlCatalogue.medium(paper, offers: $0.field, stock: stock, digitalReference: .autoLevels)
        }.map { $0.field.rawValue }]
    }
    let extensions: [String: Any] = ["stage": PipelineStage.allCases.map { $0.rawValue },
        "seed": "UInt64", "frameIndex": "UInt64", "textureStages": "Int32 mask",
        "sceneIlluminantSpectrum": "81 Float32 samples", "sceneIlluminantChromaticity": "2 Float32 xy",
        "sceneIlluminantKelvin": "Float32", "layeredTransport": "original Codable LayeredTransport",
        "scalars": ["exposureEV", "sceneHeadroom", "hdrRange", "hdrRollOff", "highlights", "shadows", "saturation", "vibrance", "grainScale", "halationScale", "halationSourceColour", "antiHalationScale", "baseThicknessScale", "pressurePlateReflectance", "flareScale", "cameraPreflash", "couplerScale", "couplerRangeScale", "couplerSelfScale", "expiredYears", "developmentEV", "bleachBypass", "printCorrection", "printerPreflash", "frameCoverage", "screenGrade", "screenExposureEV", "labScanDodging", "labScanLook", "grainMottleShare", "grainMottleSizeRatio", "halationReturnRatio", "halationHazeMM", "chromaticFringeAmount", "chromaticFringeRadiusMM", "shutterSeconds", "printViewingKelvin", "sceneHighlightStops", "sceneIlluminantKelvin", "focalLengthMM"],
        "flags": ["localTone", "useEstimatedHalationProfile"],
        "arrays": ["halationReturnGain", "couplerGapReachScales", "sceneIlluminantSpectrum"]]
    let menus = Dictionary(uniqueKeysWithValues: EditorDynamicMenu.allCases.map { menu in
        (menu.rawValue, dynamicChoices(menu, stock: stock).map { choice -> [String: Any] in
            ["id": choice.id, "label": choice.label, "detail": choice.detail, "value": choice.value.map { $0 as Any } ?? NSNull()]
        })
    })
    let composed: Set<EditorControlField> = [.gauge, .paper, .sceneLight, .sceneLightKelvin, .metering,
        .printerEnabled, .printerLamp, .printerExposure, .printerMagenta, .printerYellow, .couplerRedGreen, .couplerGreenBlue]
    let simulationControls = EditorControlCatalogue.all.filter {
        ($0.binding != nil || composed.contains($0.field)) && $0.field != .stage
    }.map { $0.field.rawValue }
    func encoded<T: Encodable>(_ value: T) throws -> Any { try JSONSerialization.jsonObject(with: JSONEncoder().encode(value), options: [.fragmentsAllowed]) }
    let defaults = FotufilmEngine.Options()
    let format = defaults.format
    let nativeDefaults: [String: Any] = ["whiteBalance": try encoded(defaults.whiteBalance),
        "grade": try encoded(defaults.grade), "printer": try encoded(PrinterProfile.simulatedTungsten),
        "receiverBands": try encoded(defaults.receiverBands), "diffusionFilter": try encoded(DiffusionFilter.preset(.proMist, grade: .quarter)),
        "grainModel": defaults.grainModel.rawValue, "adjacencyModel": AdjacencyModel.gaussian.rawValue,
        "enlarger": defaults.enlarger.rawValue, "digitalReference": defaults.digitalReference.rawValue,
        "paper": PrintPaper.default(for: stock).id, "negativeViewing": NegativeViewing.lightBox.rawValue,
        "gradeSpace": defaults.gradeSpace.rawValue, "lensFilters": ["filters": [], "compensation": LensFilterCompensation.throughTheLens.rawValue, "lensFrontReflectance": 0.005],
        "format": ["name": format.name, "frameHeightMM": format.frameHeightMM, "frameAspectRatio": format.frameAspectRatio, "thicknessMM": format.base.thicknessMM, "refractiveIndex": format.base.refractiveIndex],
        "filmGrain": ["layers": [1, 1, 1], "colour": defaults.filmGrain.colour, "size": defaults.filmGrain.size, "softness": defaults.filmGrain.softness],
        "printFinish": ["gains": [1, 1, 1], "highlights": 0, "shadows": 0, "saturation": 1, "vibrance": 0],
        "vectors": ["screenCMY": [0, 0, 0]], "layeredTransport": try encoded(stock.layeredTransport ?? LayeredTransport.illustrative(stock: stock, format: format)),
        "scanReading": ["border": [1, 1, 1], "gains": [1, 1, 1]], "unexposedEdge": ["left": 0, "right": 0, "top": 0, "bottom": 0, "carrier": 0]]
    return try JSONSerialization.data(withJSONObject: ["manifest": manifest, "stockApplicability": applicable,
        "surfaces": surfaces, "media": media, "menus": menus, "nativeExtensions": extensions,
        "simulationControls": simulationControls, "nativeCarrier": "alignment", "nativeDefaults": nativeDefaults,
        "controlCarrier": "controls", "nativeStructured": ["whiteBalance", "grade", "printer", "receiverBands", "diffusionFilter", "grainModel", "adjacencyModel", "enlarger", "digitalReference", "paper", "negativeViewing", "gradeSpace", "lensFilters", "format", "filmGrain", "printFinish", "vectors", "scanReading", "unexposedEdge"]], options: [.sortedKeys])
}

func validateAlignmentKeys(_ data: Data) throws {
    let root = try JSONSerialization.jsonObject(with: data) as! [String: Any]
    guard let native = root["alignment"] as? [String: Any] else {
        if root["alignment"] != nil { throw AlignmentFailure("alignment must be an object") }
        return
    }
    let allowed: Set<String> = ["stage", "seed", "frameIndex", "textureStages", "sceneIlluminantKelvin", "sceneIlluminantChromaticity", "sceneIlluminantSpectrum", "layeredTransport", "halationModel", "scalars", "flags", "arrays", "whiteBalance", "grade", "printer", "receiverBands", "diffusionFilter", "grainModel", "adjacencyModel", "enlarger", "digitalReference", "paper", "negativeViewing", "gradeSpace", "lensFilters", "format", "filmGrain", "printFinish", "vectors", "scanReading", "unexposedEdge"]
    if let key = native.keys.first(where: { !allowed.contains($0) }) { throw AlignmentFailure("Unknown alignment key: \(key)") }
    // These keys mirror the original Codable stored properties; typed decoding checks values.
    let objects: [String: Set<String>] = [
        "format": ["name", "frameHeightMM", "frameAspectRatio", "thicknessMM", "refractiveIndex"],
        "filmGrain": ["layers", "colour", "size", "softness"],
        "printFinish": ["gains", "highlights", "shadows", "saturation", "vibrance"],
        "lensFilters": ["filters", "compensation", "lensFrontReflectance"],
        "whiteBalance": ["kelvin", "tint"],
        "printer": ["lampKelvin", "exposureEV", "magenta", "yellow"],
        "receiverBands": ["red", "green", "blue"],
        "grade": ["shadows", "midtones", "highlights"],
        "scanReading": ["border", "gains"],
        "unexposedEdge": ["left", "right", "top", "bottom", "carrier", "fNumber", "sceneHighlightStops", "sceneChannelMedians", "sceneToneStops"],
        "diffusionFilter": ["id", "name", "interactedFraction", "albedo", "particleRadiusUM", "sizeSpread", "substrate", "coating"],
        "layeredTransport": ["revision", "kind", "constructionID", "wavelengthsNM", "layers", "frontIndex", "rearIndex", "rearReflectance", "rearPlateReflectance", "recordDepthMM", "angularExponent", "captureProbability", "returnedToDirect", "coreSigmaMM"]
    ]
    func check(_ value: Any?, keys: Set<String>, path: String) throws {
        if let object = value as? [String: Any], let key = object.keys.sorted().first(where: { !keys.contains($0) }) {
            throw AlignmentFailure("Unknown \(path) key: \(key)")
        }
    }
    for (field, keys) in objects {
        try check(native[field], keys: keys, path: "alignment.\(field)")
    }
    if let grade = native["grade"] as? [String: Any] {
        for band in ["shadows", "midtones", "highlights"] {
            try check(grade[band], keys: ["balanceX", "balanceY", "level"], path: "alignment.grade.\(band)")
        }
    }
    if let diffusion = native["diffusionFilter"] as? [String: Any] {
        try check(diffusion["substrate"], keys: ["a", "b"], path: "alignment.diffusionFilter.substrate")
    }
    if let stack = native["lensFilters"] as? [String: Any], let filters = stack["filters"] as? [Any] {
        for (index, filter) in filters.enumerated() {
            let path = "alignment.lensFilters.filters[\(index)]"
            try check(filter, keys: ["id", "name", "internalTransmittance", "substrate", "coating"], path: path)
            if let object = filter as? [String: Any] {
                try check(object["substrate"], keys: ["a", "b"], path: "\(path).substrate")
            }
        }
    }
    if let transport = native["layeredTransport"] as? [String: Any], let layers = transport["layers"] as? [Any] {
        for (index, layer) in layers.enumerated() {
            try check(layer, keys: ["id", "thicknessMM", "refractiveIndex", "absorptionPerMM"], path: "alignment.layeredTransport.layers[\(index)]")
        }
    }
}

// Adapter serialization only; preparation and image measurements call original source methods.
func alignmentPrepare(_ requestData: Data, scene: [Float]) throws -> Data {
    let decoder = JSONDecoder()
    struct Kind: Decodable { var kind: String? }
    if try decoder.decode(Kind.self, from: requestData).kind == "alignment-catalogue" {
        return try alignmentCatalogue(requestData)
    }
    let validationOnly = try decoder.decode(Kind.self, from: requestData).kind == "alignment-validate"
    try validateAlignmentKeys(requestData)
    let request = try decoder.decode(WebProfileRequest.self, from: requestData)
    let settings = try decoder.decode(AlignmentEnvelope.self, from: requestData).alignment
    guard request.controls["stage"] == nil else {
        throw AlignmentFailure("Use alignment.stage; original web stage menu does not resolve dynamic stage choices.")
    }
    var object = try JSONSerialization.jsonObject(with: requestData) as! [String: Any]
    var controls = object["controls"] as? [String: Any] ?? [:]
    let stockForMenus = try request.stock.validated().stock
    var nativeMenus: [(EngineBinding, EditorControlValue)] = []
    for control in EditorControlCatalogue.all {
        guard case .menu(.dynamic(let menu)) = control.kind,
              let input = controls[control.field.rawValue], let binding = control.binding else { continue }
        guard let id = input as? String,
              let choice = dynamicChoices(menu, stock: stockForMenus).first(where: { $0.id == id }) else {
            throw AlignmentFailure("Unknown native dynamic choice: \(control.field.rawValue)")
        }
        let value: EditorControlValue
        if binding == .shutterSeconds || binding == .printViewingKelvin {
            value = .optionalNumber(choice.value)
        } else { value = .choice(Int(choice.value ?? 0)) }
        nativeMenus.append((binding, value))
        controls.removeValue(forKey: control.field.rawValue)
    }
    object["controls"] = controls
    let configuredRequest = try decoder.decode(WebProfileRequest.self, from: JSONSerialization.data(withJSONObject: object))
    let (stock, originalOptions) = try configuredRequest.configured()
    var options = originalOptions
    for (binding, value) in nativeMenus { binding.apply(value, to: &options) }
    if let stage = settings?.stage {
        guard let parsed = PipelineStage(rawValue: stage) else { throw AlignmentFailure("Unknown alignment.stage") }
        options.stage = parsed
    }
    if let seed = settings?.seed { options.seed = seed }
    if let mask = settings?.textureStages { options.textureStages = TextureStages(rawValue: mask) }
    if let kelvin = settings?.sceneIlluminantKelvin { options.sceneIlluminantKelvin = kelvin }
    if let xy = settings?.sceneIlluminantChromaticity {
        guard xy.count == 2 else { throw AlignmentFailure("Scene illuminant chromaticity requires two values") }
        options.sceneIlluminantChromaticity = SIMD2(xy[0], xy[1])
    }
    if let spectrum = settings?.sceneIlluminantSpectrum { options.sceneIlluminantSpectrum = spectrum }
    if let construction = settings?.layeredTransport { options.layeredTransport = construction }
    if let model = settings?.halationModel { options.halationModel = model }
    try applyNativeOptions(settings, to: &options)
    if let scan = settings?.scanReading {
        let gains = scan.gains ?? [1, 1, 1]
        guard scan.border.count == 3, gains.count == 3 else { throw AlignmentFailure("scanReading requires three border and gain components") }
        options.scanReading = try ApproximateNegativeScan(stock: stock, border: SIMD3(scan.border[0], scan.border[1], scan.border[2]), gains: SIMD3(gains[0], gains[1], gains[2]))
    }
    if let edge = settings?.unexposedEdge {
        func vector(_ values: [Float]?) throws -> SIMD3<Float>? {
            guard let values else { return nil }
            guard values.count == 3 else { throw AlignmentFailure("Unexposed edge requires three measurement components") }
            return SIMD3(values[0], values[1], values[2])
        }
        options.unexposedEdge = try UnexposedEdge.Develop(margins: .init(left: edge.left, right: edge.right, top: edge.top, bottom: edge.bottom, carrier: edge.carrier ?? 0), sceneHighlightStops: edge.sceneHighlightStops, sceneChannelMedians: vector(edge.sceneChannelMedians), sceneToneStops: vector(edge.sceneToneStops), fNumber: edge.fNumber)
    }
    let frameIndex = settings?.frameIndex ?? 0
    let width = request.width, height = request.height
    guard width > 0, height > 0, width <= 120000, height <= 120000,
          Int64(width) * Int64(height) <= 120000000 else { throw AlignmentFailure("Invalid dimensions") }
    if options.stage.readsScene && !validationOnly {
        guard scene.count == width * height * 4, scene.allSatisfy(\.isFinite) else {
            throw AlignmentFailure("Exact preparation requires width*height*4 finite linear Rec.2020 straight RGBA values")
        }
    }
    var plain = stock
    let construction = options.transportConstruction(for: stock)
    if construction != nil { plain.layeredTransport = nil }
    let invocationOptions = construction == nil ? options : options.withoutLayeredTransport
    var invocation = try FilmEngineInvocation(validating: plain, options: invocationOptions,
        width: width, height: height, frameIndex: frameIndex, bindsFilmGrain: !validationOnly)
    if validationOnly {
        if let construction { try construction.validate() }
        return Data("{\"valid\":true}".utf8)
    }
    if options.stage.readsScene {
        if invocation.sceneMeteringActive {
            var measurement = invocation.toneBaseMeasurement()
            scene.withUnsafeBufferPointer { measurement.add(linearRGBA: $0.baseAddress!, rows: 0..<height) }
            invocation.setToneBase(measurement)
        }
    }
    var components: [TransportRenderPlan.Component] = []
    var heads: [FilmEngineInvocation] = []
    var tail: FilmEngineInvocation?
    if var model = construction {
        // Original expose sequence: texture removes unselected construction terms before compile.
        if options.stage == .texture && !options.textureStages.contains(.emulsionMTF) {
            model.coreSigmaMM = [0, 0, 0]
        }
        var planOptions = options
        planOptions.stage = .full
        planOptions.localTone = false
        planOptions.layeredTransport = model
        if options.stage == .texture && !options.textureStages.contains(.halation) { planOptions.halationScale = 0 }
        let plan = try LayeredTransportRenderer.renderPlan(stock: plain, options: planOptions,
                                                          width: width, height: height)
        components = plan.components
        // Same head/tail mutations as source LayeredTransportRenderer.expose, lines339-347/389-392.
        for component in components {
            var head = invocation
            head.featureMask &= FilmEngineFeature.flare | FilmEngineFeature.diffusion
            head.featureMask |= FilmEngineFeature.lightOut
            head.clearTransportOptics(keepLens: true)
            head.sharePreflash(among: components.count)
            head.setTransportExposure(SpectralLUT(dimension: invocation.spectral.exposure.dimension,
                                                   values: component.exposure))
            heads.append(head)
        }
        var continuation = invocation
        continuation.featureMask &= ~LayeredTransportRenderer.continuationClears
        continuation.clearTransportOptics(keepLens: false)
        continuation.configuration[Int(FOTUFILM_CONFIG_RECORD_INPUT)] = 1
        if options.stage == .texture { continuation.featureMask |= FilmEngineFeature.densityOut }
        tail = continuation
    }
    let tiles = invocation.filmTileBinding?.packed() ?? []
    let lutCount = invocation.spectral.exposure.values.count
    var data = Data("FSA1".utf8)
    // Version 2 header: fourteen 32-bit fields; explicit plan flag includes zero-component plans.
    for value in [2, width, height, Int(invocation.featureMask), Int(truncatingIfNeeded: invocation.seed),
                  invocation.configuration.count, invocation.spectral.exposure.dimension, lutCount,
                  invocation.spectral.paperOutput == nil ? 0 : 1, tiles.count, components.count,
                  invocation.spatialSupport, stock.donorLayers.isEmpty ? 3 : 4, tail == nil ? 0 : 1] {
        data.appendUInt32(UInt32(truncatingIfNeeded: value))
    }
    data.appendFloats(invocation.configuration)
    data.appendFloats(invocation.spectral.exposure.values)
    data.appendFloats(invocation.spectral.filmOutput.values)
    if let paper = invocation.spectral.paperOutput { data.appendFloats(paper.values) }
    data.appendFloats(tiles)
    if let tail {
        data.appendInt32(tail.featureMask); data.appendUInt32(tail.seed)
        data.appendFloats(tail.configuration)
        for (index, component) in components.enumerated() {
            let head = heads[index]
            data.appendInt32(head.featureMask); data.appendUInt32(head.seed)
            data.appendInt32(Int32(component.exposure.count)); data.appendInt32(Int32(component.stencils.count))
            data.appendFloats(head.configuration)
            data.appendFloats(component.exposure); data.appendFloats(component.stencils)
        }
    }
    return data
}

#if os(WASI)
private var input: UnsafeMutablePointer<UInt8>?
private var inputCount = 0
private var output: UnsafeMutablePointer<UInt8>?
private var outputCount = 0
private var sceneInput: UnsafeMutablePointer<Float>?
private var sceneCount = 0

@_expose(wasm, "profile_input")
@_cdecl("profile_input")
func profileInput(_ count: Int) -> UnsafeMutablePointer<UInt8>? {
    input?.deallocate(); input = nil; inputCount = 0
    guard count > 0, count <= 8 * 1024 * 1024 else { return nil }
    input = .allocate(capacity: count); inputCount = count
    return input
}
@_expose(wasm, "align_scene_input")
@_cdecl("align_scene_input")
func alignSceneInput(_ count: Int) -> UnsafeMutablePointer<Float>? {
    sceneInput?.deallocate(); sceneInput = nil; sceneCount = 0
    guard count >= 0, count <= 480000000 else { return nil }
    if count > 0 { sceneInput = .allocate(capacity: count) }
    sceneCount = count
    return sceneInput
}
func prepareResponse(aligned: Bool) -> Int32 {
    output?.deallocate(); output = nil; outputCount = 0
    let bytes: Data
    let status: Int32
    do {
        guard let input, inputCount > 0 else { throw AlignmentFailure("Missing request JSON") }
        let request = Data(bytes: input, count: inputCount)
        if aligned {
            let scene = sceneInput.map { Array(UnsafeBufferPointer(start: $0, count: sceneCount)) } ?? []
            bytes = try alignmentPrepare(request, scene: scene)
        } else { bytes = try WebRenderRequest.prepare(request) }
        status = 0
    } catch { bytes = Data(String(describing: error).utf8); status = 1 }
    input?.deallocate(); input = nil; inputCount = 0
    sceneInput?.deallocate(); sceneInput = nil; sceneCount = 0
    output = .allocate(capacity: bytes.count); outputCount = bytes.count
    bytes.copyBytes(to: output!, count: bytes.count)
    return status
}
@_expose(wasm, "profile_prepare")
@_cdecl("profile_prepare")
func profilePrepare() -> Int32 { prepareResponse(aligned: false) }
@_expose(wasm, "align_prepare")
@_cdecl("align_prepare")
func alignPrepare() -> Int32 { prepareResponse(aligned: true) }
@_expose(wasm, "profile_output")
@_cdecl("profile_output")
func profileOutput() -> UnsafeMutablePointer<UInt8>? { output }
@_expose(wasm, "profile_output_size")
@_cdecl("profile_output_size")
func profileOutputSize() -> Int { outputCount }
#else
do {
    let request = FileHandle.standardInput.readDataToEndOfFile()
    if CommandLine.arguments.contains("--align") {
        let path = CommandLine.arguments.dropFirst(2).first
        let sceneData = try path.map { try Data(contentsOf: URL(fileURLWithPath: $0)) } ?? Data()
        guard sceneData.count % 4 == 0 else { throw AlignmentFailure("Scene file size must be Float32 aligned") }
        var scene = [Float](repeating: 0, count: sceneData.count / 4)
        scene.withUnsafeMutableBytes { sceneData.copyBytes(to: $0) }
        FileHandle.standardOutput.write(try alignmentPrepare(request, scene: scene))
    } else if CommandLine.arguments.contains("--catalogue") {
        FileHandle.standardOutput.write(try WebProfileCatalogue.data(JSONDecoder().decode([String: FilmStockDefinition].self, from: request)))
    } else { FileHandle.standardOutput.write(try WebRenderRequest.prepare(request)) }
} catch {
    FileHandle.standardError.write(Data("\(error)\n".utf8)); exit(1)
}
#endif
