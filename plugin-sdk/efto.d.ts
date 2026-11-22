// efto 插件 API 类型声明（字段名与运行时一致）。
// 用法：在插件目录放一份，或 tsconfig.json 里 "types": ["./efto.d.ts"]。
// TS 只做类型剥离：不能使用 enum / 带值 namespace / 装饰器 / JSX。

declare const efto: Efto;

type MaybePromise<T> = T | Promise<T>;

interface DbDoc {
  _id: string;
  _rev?: string;
  [key: string]: unknown;
}

interface AllDocsOptions {
  prefix?: string;
  startkey?: string;
  endkey?: string;
  descending?: boolean;
  limit?: number;
  skip?: number;
  include_docs?: boolean;
}

interface AllDocsResult {
  total_rows: number;
  offset: number;
  rows: Array<{ id: string; key: string; value: { rev?: string }; doc?: DbDoc }>;
}

interface ChangeEmitter {
  onChange(cb: (e: { id: string; doc: DbDoc | null; type: 'create' | 'update' | 'delete' }) => void): ChangeEmitter;
  removeListener(cb: unknown): ChangeEmitter;
  start(): ChangeEmitter;
  stop(): ChangeEmitter;
}

interface PluginEnterInfo {
  /** 命中的关键字 */
  code: string;
  type: string;
  /** 关键字之后的输入内容 */
  payload: unknown;
  msg: unknown;
}

interface EftoDb {
  put(doc: DbDoc): { ok: boolean; id: string; rev: string };
  bulkPut(docs: DbDoc[]): Array<{ ok: boolean; id: string; rev: string }>;
  get(id: string): DbDoc | null;
  bulkGet(ids: string[]): Array<DbDoc | null>;
  remove(docOrId: DbDoc | string): { ok: boolean; id: string };
  allDocs(opts?: AllDocsOptions): AllDocsResult;
  info(): { db_name: string; doc_count: number; update_seq: number };
  changes(opts?: { since?: number }): { last_seq: number; results: unknown[] };
  changeEmitter(): ChangeEmitter;
}

interface EftoKv {
  get(key: string): unknown;
  set(key: string, value: unknown): boolean;
  assign(key: string, value: unknown): boolean;
  remove(key: string): boolean;
  all(): Record<string, unknown>;
  keys(): string[];
  size(): { keys: number; size: number };
  read(key: string, cb?: (v: unknown) => void): unknown;
  write(key: string, value: unknown, cb?: (r: unknown) => void): unknown;
}

interface EftoExecOptions {
  /** 白名单里的程序名，按平台生效：'es'（Everything CLI，仅 Windows）、
   * 'mdfind'（Spotlight，仅 macOS）。平台外调用明确报"当前平台不支持"，不会静默成功 */
  name: string;
  /** 参数数组，原样交给 CreateProcess —— 不经过任何 shell */
  args?: string[];
  /** 超时毫秒，默认 3000，上限 15000；超时会 kill 子进程 */
  timeoutMs?: number;
}

interface EftoExecResult {
  ok: true;
  code: number;
  stdout: string;
  stderr: string;
  path: string;
  truncated: boolean;
}

interface EftoScriptOptions {
  /** 脚本源码，通过 node -e / python -c 执行 */
  code: string;
  args?: string[];
  timeoutMs?: number;
}

interface EftoScriptResult {
  runtime: 'node' | 'python';
  ok: boolean;
  code: number;
  stdout: string;
  stderr: string;
  path: string;
  truncated: boolean;
}

interface EftoHostsRead {
  path: string;
  backup: string;
  text: string;
  encoding: string;
  bytes: number;
}

interface EftoAiTextPart {
  type: 'text';
  text: string;
}

/** 本机图片路径；由宿主读盘并转成 data URL，插件拿不到文件字节。 */
interface EftoAiImagePart {
  type: 'image';
  path: string;
}

type EftoAiContent = string | Array<EftoAiTextPart | EftoAiImagePart>;

interface EftoAiMessage {
  role: 'system' | 'user' | 'assistant';
  content: EftoAiContent;
}

interface EftoAiChatOptions {
  messages: EftoAiMessage[];
  model?: string;
  temperature?: number;
  maxTokens?: number;
  timeoutSecs?: number;
  /** 为 true 时走 SSE 流式接口；传 onDelta 时自动开启 */
  stream?: boolean;
  /** 流式增量：`text` 是正文，`reasoning` 是思考过程（模型支持时） */
  onDelta?: (delta: EftoAiDelta) => void;
  /** 开启 DeepSeek 服务端联网搜索（需要 deepseek-flash 模型） */
  webSearch?: boolean;
  /** 开启 DeepSeek 服务端网页抓取（自动读取消息里的链接） */
  readUrl?: boolean;
}

interface EftoAiDelta {
  text?: string;
  reasoning?: string;
  /** 联网进度：`{ type: 'call_started' | 'call_completed' | 'step_reasoning_done', call?: { action?: { type: string }, kind?: string } }` */
  toolEvent?: unknown;
}

/** DeepSeek 联网工具的一次调用（`message.server_tool_calls` 的一步） */
interface EftoAiToolStep {
  index: number;
  reasoning_content?: string;
  calls: Array<{
    index: number;
    id: string;
    type: 'deepseek_web_search' | 'deepseek_read_url';
    kind: string;
    action?: { type: string; queries?: string[]; url?: string; pattern?: string };
    status: 'completed' | 'failed';
    sources?: Array<{ type: string; url: string; title?: string; snippet?: string; site_name?: string }>;
    page?: { url: string; title?: string; published_at?: string };
    error?: { code?: string };
  }>;
}

interface EftoAiAnnotation {
  type: 'url_citation';
  url_citation: { url: string; title?: string; start_index?: number; end_index?: number };
}

interface EftoAiChatResult {
  content: string;
  /** 思考过程（模型支持时；非流式请求为空字符串） */
  reasoning: string;
  model: string;
  modelId: string;
  modelName: string;
  finishReason: string;
  usage: unknown;
  /** 联网工具调用记录；未开启联网时为 null */
  serverToolCalls: EftoAiToolStep[] | null;
  /** 正文里的引用；未开启联网或没有引用时为 null */
  annotations: EftoAiAnnotation[] | null;
}

interface EftoAiOcrOptions {
  /** 本机文件路径（PDF / Word / PowerPoint / 图片） */
  path: string;
  /** 默认 text_recognition；rich_layout_recognition 会保留版面与图片 */
  purpose?: 'text_recognition' | 'rich_layout_recognition';
  model?: string;
}

interface EftoAiOcrResult {
  id: string;
  purpose: string;
  pageCount: number;
  fileName: string;
  pages: Array<{ index: number; text: string; finishReason: 'stop' | 'length' }>;
}

interface EftoModels {
  default: string;
  models: Array<{ id: string; name: string; model: string; base_url: string; has_api_key: boolean }>;
}

interface EftoOutputRow {
  title: string;
  subtitle?: string;
  /** 回车时复制到剪贴板 */
  copy?: string;
  /** 回车时交给系统打开（文件/目录/URL） */
  open?: string;
}

/**
 * 插件 API。main 进程与 UI 页面共用同名对象：
 * - main：`efto.exec` / `efto.node` / `efto.ai` 等全部能力；
 * - UI：宿主注入的 SDK 只提供通信、kv、窗口与系统能力（见 assets/plugin-sdk.js）。
 */
interface EftoSearchItem {
  /** 搜索结果标题（也用作命中后的 code） */
  title: string;
  /** 回车时回传给 onPluginEnter 的 info.code */
  cmd: string;
}

interface Efto {
  readonly platform: 'win32';
  readonly version: string;
  readonly locale: string;
  readonly dbPath: string;
  readonly oneId: string;

  /** 文档型存储 */
  db: EftoDb;
  /** 键值存储 */
  kv: EftoKv;

  /** 受限外部程序执行（白名单，目前仅 es） */
  exec(opts: EftoExecOptions): Promise<EftoExecResult>;
  /** 列表式输出，最多 50 行 */
  output(rows: EftoOutputRow[]): boolean;
  /** main → UI 页面 */
  ipcMessage(name: string, data?: unknown): Promise<unknown>;
  /** Node.js 脚本（需在 设置 → 运行时 配置解释器） */
  node: { run(opts: EftoScriptOptions): Promise<EftoScriptResult> };
  /** Python 脚本（需在 设置 → 运行时 配置解释器） */
  python: { run(opts: EftoScriptOptions): Promise<EftoScriptResult> };
  /** 系统 hosts 文件（Windows %SystemRoot%\...\hosts / macOS /etc/hosts）。
   * 保存时提权（Windows UAC / macOS 管理员授权）；其它平台调用明确报
   * "当前平台不支持：hosts" */
  hosts: {
    path(): string;
    read(): EftoHostsRead;
    save(opts: { text: string; encoding?: string }): Promise<{ ok: boolean; path: string; backup: string; bytes: number }>;
  };
  /** 大模型（需在 设置 → 模型 配置并设为默认） */
  ai: {
    chat(opts: EftoAiChatOptions): Promise<EftoAiChatResult>;
    models(): EftoModels;
    /** DeepSeek OCR：把本机文件识别成逐页文字（文件不会交给对话模型） */
    ocr(opts: EftoAiOcrOptions): Promise<EftoAiOcrResult>;
  };
  /**
   * 把插件自己的数据挂到启动器搜索。
   * 回车后 cmd 会作为 `info.code` 回传给 onPluginEnter；宿主会落盘，重启后仍能搜到。
   * 传空数组即注销。
   */
  setSearchItems(items: EftoSearchItem[]): void;

  onPluginEnter(cb: (info: PluginEnterInfo) => void): (info: PluginEnterInfo) => void;
  onPluginOut(cb: (info?: { code?: string }) => void): unknown;
  onMainMessage(cb: (msg: { name: string; data: unknown }) => void): unknown;
  onMessage(cb: (msg: { name: string; data: unknown }) => void): unknown;
  onUiClosed(cb: () => void): unknown;
  onWindowHide(cb: () => void): unknown;
  onWindowShow(cb: () => void): unknown;
  onDatabaseChange(cb: (e: unknown) => void): unknown;
  removeListener(event: string, cb: unknown): void;

  showUI(options?: { title?: string; width?: number; height?: number }): Promise<{ ok: boolean }>;
  hideUI(): Promise<{ ok: boolean }>;
  hideMainWindow(): Promise<{ ok: boolean }>;
  showMainWindow(): Promise<{ ok: boolean }>;
  setSubTitle(text: string): boolean;
  closeWindow(name?: string): boolean;
  /** UI 页面专用：隐藏当前插件窗口 */
  hideWindow(): Promise<unknown>;

  redirect(label: string, options?: Record<string, unknown>): Promise<unknown>;
  broadcast(channel: string, data?: unknown): boolean;
  sendToDefaultOutput(payload: unknown): boolean;
  sendToFocusWindow(text: string): boolean;

  copyText(text: string): boolean;
  pasteText(): string;
  /** 读剪贴板里的位图（main 进程可用），落成临时 PNG 并返回绝对路径；没有图片时返回空字符串。 */
  pasteImage(): string;
  /** 本地图片预览（main 进程可用）：返回 data URL（大图会缩到 640px 内），空路径返回空字符串。 */
  imagePreview(path: string): string;
  showNotification(text: string, sub?: string): boolean;
  hideNotification(): boolean;
  shellOpenPath(path: string): boolean;
  shellShowItemInFolder(path: string): boolean;
  openExternal(url: string): boolean;
  getPathByName(name: 'home' | 'desktop' | 'documents' | 'downloads' | 'data' | 'temp'): string;
  getPathForFile(file: unknown): string;
  quit(): boolean;
  relaunch(): never;

  /** UI 页面专用：原生文件选择框，取消时返回空数组 */
  pickFiles(): Promise<string[]>;
  /** UI 页面专用：系统文件拖入松手 */
  onFileDrop(cb: (paths: string[]) => void): unknown;
  /** UI 页面专用：拖拽进入/离开高亮 */
  onFileDrag(cb: (inside: boolean) => void): unknown;

  isDarkMode(): boolean;
  isLightMode(): boolean;
  isWindows(): boolean;
  isMacOS(): boolean;
  isLinux(): boolean;
  isMainProcess?(): boolean;
}

/** UI 页面里由宿主注入的 SDK（与 main 同名，实现走 IPC 命令）。 */
interface Window {
  efto: Efto;
}
