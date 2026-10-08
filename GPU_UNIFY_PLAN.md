# spektrafilm-rs CPU/GPU 统一重构实施计划

状态：历史实施记录；CUDA 支持部分已被后续移除并由当前 CPU/WGPU 架构取代。本文件保留当时的原始边界、证据和验证结果；其中 CUDA 内容仅代表历史，不是当前构建或使用说明。
## 当前移除记录（2026-10-08）

- 当前架构仅保留 CPU（f64 reference/export）与 WGPU（f32 interactive preview）；Grain V2 及 CPU grain 保留。
- CUDA 后端、Cargo feature、构建/运行入口和 CUDA 打包选项已从当前实现与用户文档移除。
- 本次未运行真实 GPU parity、性能或平台打包验证；下文 CUDA 内容仅为历史证据。
- 验证通过：workspace 全 feature 编译；默认测试 180 项、precision-f64 测试 181 项；GPU crate 无默认 feature 的 f64 编译；CLI help；CPU f64 CLI 输出 2752×1536 PNG；V1、V2 Analogue、V2 Noise 在直扫/印相下共 6 个 CPU f64 渲染及 TIFF 回读场景。
- `just --list` 已无 CUDA 命令，macOS 打包脚本通过 Bash 语法检查；本机未运行 Windows PowerShell 打包。

## 历史实施计划（已 superseded）

## 1. 目标

参考 Dehancer 逆向报告，将 spektrafilm-rs 的维护边界收敛为：

```text
共享：阶段语义、参数准备、资源/LUT/cache key、执行能力判断
CPU：f64 reference/export，保留 Python parity
GPU：f32 interactive preview，WGPU/CUDA 各自负责 kernel dispatch
```

目标不是让 CPU 和 GPU 共用同一份像素级代码，而是避免同一个物理/颜色规则在多个 Rust 编排位置重复解释。

## 2. 明确不做

- 不删除 CPU f64 reference/export；它是当前 Python parity 和最终导出的契约。
- 不把 WGSL/CUDA 改写成 CPU 调用，或把 CPU 像素循环搬进 GPU 抽象。
- 不在本轮决定 CUDA 后端是否保留；该问题需要真实 NVIDIA 设备上的性能和兼容性证据。
- 不顺带统一 GUI、CLI、runtime 的 preview resize 策略。
- 不引入新的用户配置字段、兼容层或额外的效果算法。
- 不在无真实 GPU 的本机宣称 GPU parity 或性能结论。

## 3. 当前架构与主要问题

### 3.1 三种执行形态

1. `spektrafilm-core/src/stages/*` + `spektrafilm-model`：CPU reference/per-stage 路径。
2. `ComputeBackend` 操作级方法：GPU 后端只覆盖部分算子，未覆盖方法回到 CPU。
3. `Pipeline::try_gpu_resident` → `FilmChainParams` → WGPU/CUDA resident chain：一上传、一回读的预览快路径。

### 3.2 最高风险

`Pipeline::try_gpu_resident`（当前基线约 `pipeline.rs:1268-1761`）重新构建了 CPU stage 已经解释过的 film/print/scan/effect 参数。新增或修正一个参数时，容易只修改一条路径。

重复的高价值区域：

- scan illuminant、normalization、viewing white、CAT、XYZ→RGB；
- DIR matrix scaling、`curves_before_dir`、density maxima；
- glare LogNormal 参数；
- film/print density curves、spectral arrays；
- resident chain 的 stage order 与 CPU tap topology。

### 3.3 必须保留的数值差异

CPU 与 GPU 目前不是同一数值实现：

- CPU 重点是 f64 和 Python 运算顺序；GPU 是 f32、融合矩阵和 shader reductions。
- CPU halation 保留逐通道 sigma/tail/strength；GPU resident 使用近似的平均 sigma。
- CPU DIR 使用真实 exponential-tail 模型；GPU resident 使用 Gaussian 近似。
- CPU grain 支持 layered/composite Poisson-binomial；grain active 时 resident capability gate 选择 faithful CPU per-stage path，不再维护 resident grain shader。
- CPU Gaussian 在部分 sigma 区间使用 IIR；GPU 使用 FIR，并受半径上限约束。

因此“统一”只能落在语义、输入和编排，不是像素级算法。

## 4. 分阶段实施计划

### Phase 1：共享 backend-neutral preparation

#### Step 1A：共享纯数学准备（已完成）

新增/提取以下纯函数，保持输入输出语义明确，不接触 GPU buffer：

- `prepare_dir(...) -> DirPrepared`
  - `matrix_scaled`
  - `curves_0`
  - `density_max`
  - CPU `apply_density_correction` 与 resident builder 共用。
- `lognormal_params(percent, roughness) -> (mu, sigma)`
  - CPU glare sampler 与 resident glare params 共用。
- `ScanColorContext`
  - resolved illuminant
  - `n_wl`
  - normalization
  - illuminant XYZ
  - viewing white
  - CAT matrix
  - output XYZ→RGB matrix
- glare RGB offset preparation
  - 明确 CPU 的两步 CAT→RGB 顺序。
  - resident GPU 是否采用组合矩阵必须单独标注为 preview narrowing/approximation，不隐藏语义差异。

实施原则：先逐行搬运现有 CPU reference 公式；禁止在抽取时优化矩阵、改变求和顺序或改变 epsilon。

#### Step 1B：统一两条调用路径（已完成）

- `stages/scanning.rs` 使用 `ScanColorContext`。
- `pipeline.rs` resident preparation 使用同一 context。
- `stages/filming.rs` 使用 `prepare_dir` 提供的纯准备结果；图像处理、diffusion、density interpolation 仍留在原模型函数。
- `pipeline.rs` 只负责把 prepared values 转为 `FilmChainParams` 和 GPU 参数结构。

CPU tap 行为必须保持不变：`RgbIn → RgbPre → LogEFilm → CmyFilm → LogEPrint → CmyPrint → RgbOut`，以及 direct film scan 的短拓扑不变。

#### Step 1C：清理不可达 resident grain（已完成）

`try_gpu_resident` 在 grain active 时已经选择 per-stage CPU 路径，因此 resident grain 参数和 dispatch 不属于可达的正常 pipeline。已删除：

- `FilmChainParams.grain` 和 `GrainGpuParams`；
- WGPU/CUDA resident grain state、kernel wiring；
- `spektrafilm-shaders/wgsl/grain.wgsl` 和 `spektrafilm-shaders/cuda/grain.cu`。

独立的 `film_grain_v2` API、CPU grain model 和 CPU per-stage grain sampling 保留。

### Phase 2：合并 WGPU/CUDA 纯 Rust helpers（已完成）

只合并不依赖后端资源的 helper：

- Scalar/f64→f32 conversion；
- spectral NaN sanitization；
- curve flattening；
- uniform-grid detection；
- FIR radius/blur support predicate。

保留后端专属代码：

- WGPU bind-group layout、pipeline cache、command encoding；
- CUDA module loading、device allocation、kernel launch；
- WGSL/CUDA struct alignment 和 buffer lifetime。

不得把 WGPU/CUDA 的不同 shader layout 强行抽成 pass-through abstraction。

### Phase 3：集中执行策略（已完成）

新增一个明确的 resident capability decision，返回结构化结果，而不是在 `try_gpu_resident` 内散落 `return None`：

```text
ResidentDecision::UseResident
ResidentDecision::PerStage { reasons: Vec<FallbackReason> }
```

候选 `FallbackReason`：

- input transfer decoding；
- requested scanner/enlarger PCHIP LUT；
- active optical diffusion；
- faithful grain distribution；
- unsupported output gamut; 
- blur radius exceeds backend support；
- missing GPU resident front pass。

现有行为保持：fallback 仍然是 per-stage CPU/reference 路径；不能为了减少分支而静默截断 blur 或替换精确效果。

日志统一输出：backend、execution mode、fallback reasons。

### Phase 4：验证与文档（已完成）

#### CPU-only 本机已验证

- `cargo check --workspace`；
- `cargo check -p spektrafilm-gpu --features cuda-backend`；
- `cargo test -p spektrafilm-core -p spektrafilm-gpu -p spektrafilm-model --features precision-f64`：173 tests passed；
- resident fallback 日志现在统一包含 backend、execution mode 和 `fallback_reasons`；
- README 已移除 resident grain shader 说明，并明确 grain 走 faithful CPU per-stage path。

未宣称真实 GPU parity、shader 编译、设备 limits 或性能：当前环境没有可用的真实 GPU 证据。

#### 必须在真实 GPU/CI 验证

- WGPU resident chain 端到端；
- CUDA resident chain 端到端；
- GPU/CPU max/mean error；
- shader pipeline compilation；
- upload/dispatch/readback 性能；
- 大图 storage buffer 和设备 limits；
- Metal/Vulkan/DX12/NVIDIA 差异。

- 当前 WGPU 初始化使用 `force_fallback_adapter: false`（`crates/spektrafilm-gpu/src/wgpu_backend.rs:72-76`），本机无 GPU 时不能把软件 adapter 或 CPU tests 当成真实 GPU 证据。

## 5. 验收标准

### 语义与维护

- film/print/scan 的共享准备逻辑只有一个权威实现。
- CPU stages 和 resident builder 不再各自重算同一组派生量。
- WGPU/CUDA 共享纯数据准备，但保留后端资源和 dispatch 边界。
- fallback 原因可从日志/结构化执行状态直接识别。

### 行为

- CPU f64 parity 测试不回退。
- CPU export 输出不被 GPU backend selection 影响。
- 不支持 resident 的效果继续走 faithful CPU path。
- GPU preview 允许明确记录的 f32/近似误差，不冒充 reference parity。
- grain、DIR、halation 的 approximation 不被描述成 CPU 等价实现。

### 代码质量

- 不保留不可达 resident grain 代码。
- 不新增仅转发参数的多层 wrapper。
- 每个阶段完成后可单独编译和测试。
- 变更只涉及本计划范围。

## 6. 风险与控制

| 风险 | 控制措施 |
|---|---|
| scan 公式是 Python parity 敏感区 | 逐行移动 f64 公式，禁止改变求和/除法顺序 |
| GPU 与 CPU 现有运算顺序不同 | 把差异记录为 preview contract，不强行 bit identity |
| `FilmChainParams` 是跨 crate API | 先做全仓库引用搜索，再删除字段或 kernel wiring |
| fallback 行为被意外扩大 | 每个 capability gate 保留现有场景；新增原因枚举不改变决策 |
| 无 GPU 无法做 resident smoke | 由真实 GPU CI/设备承担，CPU 本机只证明 reference 和编译 |
| 重构编辑错位 | 小范围编辑；每一步 `cargo check`；不做大块盲替换 |

## 7. 实施与验证记录

- 已建立数据盘 worktree：`/data/worktrees/spektrafilm-gpu-unify`。
- 基线：`412d8d9`。
- 使用本机 LibRaw 0.22 构建前缀完成 baseline `cargo check --workspace`。
- baseline 核心测试通过：`spektrafilm-core`、`spektrafilm-gpu`、`spektrafilm-model`，共观察到 135、2、2、31 等测试组全部通过。
- 已验证的 Step 1A 原型保存在分支：`backup/gpu-unify-prototype`，提交 `ec7a2c7`。
  - 原型验证了 `prepare_dir` 和 `lognormal_params` 的抽取可编译；
  - 核心/模型测试保持全绿。
- 随后在实施 worktree 完成正式重构：共享 `chain_prep`、GPU 纯数据 helper、`ResidentDecision`/fallback 原因，以及不可达 resident grain wiring 清理；当前变更待合并到目标分支。
- 已确认：`N_WAVELENGTHS = 81`；`colorspace_white_xyz_f64` 与 `RgbColorSpace::whitepoint_xyz` 同源。

## 8. 后续验证

1. 共享准备已落地到 `chain_prep.rs`、`couplers.rs` 和 `glare.rs`；CPU stages 与 resident builder 使用同一组派生数据。
2. `spektrafilm-gpu/src/gpu_helpers.rs` 统一 scalar 转换、NaN sanitization、curve flattening、uniform-grid 检测和矩阵转换；WGPU/CUDA 资源与 dispatch 仍保持后端专属。
3. resident capability gate 已集中为 `ResidentDecision`；CPU per-stage fallback 保留，日志记录 `fallback_reasons`。
4. 真实 GPU 可用后仍需运行 backend parity、resident smoke、shader 编译和性能验证；本机检查不构成 GPU 证据。
