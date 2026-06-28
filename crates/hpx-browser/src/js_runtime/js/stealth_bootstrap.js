// === Fingerprint Utilities ===
var _fpSeed = 0;
var _fpCache = null;

function _fpRand(salt) {
  var h = (_fpSeed ^ (salt || 0)) | 0;
  h = Math.imul(h ^ (h >>> 16), 0x45d9f3b);
  h = Math.imul(h ^ (h >>> 13), 0x45d9f3b);
  return ((h ^ (h >>> 16)) >>> 0) / 0xFFFFFFFF;
}

function _fpNoise(x, y, channel) {
  return (_fpRand(x * 7919 + y * 6271 + channel * 8923) - 0.5) * 4;
}

function _fp(name) {
  if (_fpCache && _fpCache[name] !== undefined) {
    return _fpCache[name];
  }
  var val = _fpRand(name.length * 31 + name.charCodeAt(0) * 97);
  if (!_fpCache) _fpCache = {};
  _fpCache[name] = val;
  return val;
}

// === Chrome Major Version Helper ===
function _chromeMajor() {
  var match = (globalThis.__hpx_ua || '').match(/Chrome\/(\d+)/);
  return match ? parseInt(match[1], 10) : 120;
}

// === Plugin/MimeType Stubs ===
globalThis.Plugin = function Plugin(name, description, filename) {
  this.name = name;
  this.description = description;
  this.filename = filename;
  this.length = 0;
};
globalThis.Plugin.prototype = {
  item: function(i) { return null; },
  namedItem: function() { return null; },
  [Symbol.iterator]: function() { return [][Symbol.iterator](); }
};

globalThis.MimeType = function MimeType(type, suffixes, description) {
  this.type = type;
  this.suffixes = suffixes;
  this.description = description;
  this.enabledPlugin = null;
};

globalThis.PluginArray = function PluginArray(items) {
  this.length = items ? items.length : 0;
  for (var i = 0; i < this.length; i++) {
    this[i] = items[i];
    this[items[i].name] = items[i];
  }
};
globalThis.PluginArray.prototype = {
  item: function(i) { return this[i] || null; },
  namedItem: function(name) { return this[name] || null; },
  refresh: function() {},
  [Symbol.iterator]: function() {
    var arr = [], self = this;
    for (var i = 0; i < self.length; i++) arr.push(self[i]);
    return arr[Symbol.iterator]();
  }
};

globalThis.MimeTypeArray = function MimeTypeArray(items) {
  this.length = items ? items.length : 0;
  for (var i = 0; i < this.length; i++) {
    this[i] = items[i];
    this[items[i].type] = items[i];
  }
};
globalThis.MimeTypeArray.prototype = {
  item: function(i) { return this[i] || null; },
  namedItem: function(name) { return this[name] || null; },
  [Symbol.iterator]: function() {
    var arr = [], self = this;
    for (var i = 0; i < self.length; i++) arr.push(self[i]);
    return arr[Symbol.iterator]();
  }
};

// === sec-ch-ua GREASE ===
var _GREASE_CHARS = [' ', '(', ':', '-', '.', '/', ')', ';', '=', '?', '_'];
var _GREASE_VER = ['8', '99', '24'];
var _BRAND_PERMS = [[0,1,2],[0,2,1],[1,0,2],[1,2,0],[2,0,1],[2,1,0]];

function _uaBrands() {
  var seed = _chromeMajor();
  var grease = {
    brand: 'Not' + _GREASE_CHARS[seed % 11] + 'A' + _GREASE_CHARS[(seed + 1) % 11] + 'Brand',
    version: _GREASE_VER[seed % 3],
  };
  var ordered = [
    grease,
    {brand: 'Chromium', version: String(seed)},
    {brand: 'Google Chrome', version: String(seed)},
  ];
  var p = _BRAND_PERMS[seed % 6];
  return [ordered[p[0]], ordered[p[1]], ordered[p[2]]];
}

function _greaseHeader() {
  var brands = _uaBrands();
  var parts = [];
  for (var i = 0; i < brands.length; i++) {
    var b = brands[i];
    var escaped = '';
    for (var j = 0; j < b.brand.length; j++) {
      var ch = b.brand[j];
      if (ch === '"' || ch === '\\') {
        escaped += '\\' + ch;
      } else {
        escaped += ch;
      }
    }
    parts.push('"' + escaped + '"');
    parts.push('v="' + b.version + '"');
  }
  return parts.join(', ');
}

// === userAgentData ===
function _buildUserAgentData() {
  var brands = _uaBrands();
  var chromeVer = _chromeMajor();
  var uaPlatform = globalThis.__hpx_ua_platform || 'Windows';
  var uaPlatformVersion = globalThis.__hpx_ua_platform_version || '15.0.0';

  return {
    brands: brands,
    mobile: false,
    platform: uaPlatform,
    getHighEntropyValues: function(hints) {
      return Promise.resolve({
        brands: brands,
        mobile: false,
        platform: uaPlatform,
        platformVersion: uaPlatformVersion,
        architecture: 'x86',
        bitness: '64',
        model: '',
        uaFullVersion: chromeVer + '.0.7778.168',
        fullVersionList: [{ brand: 'Google Chrome', version: chromeVer + '.0.7778.168' }],
        wow64: false
      });
    },
    toJSON: function() {
      return {
        brands: brands,
        mobile: false,
        platform: uaPlatform
      };
    }
  };
}

// === Chrome Object ===
function _buildChrome() {
  var chromeVer = _chromeMajor();

  return {
    app: {
      InstallState: {
        DISABLED: 'disabled',
        INSTALLED: 'installed',
        NOT_INSTALLED: 'not_installed'
      },
      RunningState: {
        CANNOT_RUN: 'cannot_run',
        READY_TO_RUN: 'ready_to_run',
        RUNNING: 'running'
      },
      isInstalled: false,
      InstallState: 'not_installed',
      RunningState: 'cannot_run',
      getDetails: function() { return null; },
      getIsInstalled: function() { return false; },
      installState: function(cb) { if (cb) cb('not_installed'); },
      runningState: function() { return 'cannot_run'; }
    },
    runtime: {
      OnInstalledReason: {
        CHROME_UPDATE: 'chrome_update',
        INSTALL: 'install',
        SHARED_MODULE_UPDATE: 'shared_module_update',
        UPDATE: 'update'
      },
      OnRestartRequiredReason: {
        APP_UPDATE: 'app_update',
        OS_UPDATE: 'os_update',
        PERIODIC: 'periodic'
      },
      PlatformArch: {
        ARM: 'arm',
        ARM64: 'arm64',
        MIPS: 'mips',
        MIPS64: 'mips64',
        X86_32: 'x86-32',
        X86_64: 'x86-64'
      },
      PlatformNaclArch: {
        ARM: 'arm',
        MIPS: 'mips',
        MIPS64: 'mips64',
        X86_32: 'x86-32',
        X86_64: 'x86-64'
      },
      PlatformOs: {
        ANDROID: 'android',
        CROS: 'cros',
        LINUX: 'linux',
        MAC: 'mac',
        OPENBSD: 'openbsd',
        WIN: 'win'
      },
      RequestUpdateCheckStatus: {
        NO_UPDATE: 'no_update',
        THROTTLED: 'throttled',
        UPDATE_AVAILABLE: 'update_available'
      },
      connect: function() { return { onMessage: { addListener: function() {} }, postMessage: function() {}, disconnect: function() {} }; },
      sendMessage: function() {},
      id: undefined
    },
    csi: function() {
      return {
        onloadT: Date.now() - Math.floor(Math.random() * 500),
        pageT: Math.floor(Math.random() * 5000) + 1000,
        startE: Date.now() - Math.floor(Math.random() * 10000),
        tran: 15
      };
    },
    loadTimes: function() {
      var base = Date.now() - Math.floor(Math.random() * 5000);
      return {
        requestTime: base / 1000,
        startLoadTime: (base + Math.floor(Math.random() * 100)) / 1000,
        commitLoadTime: (base + Math.floor(Math.random() * 300)) / 1000,
        finishDocumentLoadTime: (base + Math.floor(Math.random() * 800)) / 1000,
        finishLoadTime: (base + Math.floor(Math.random() * 1200)) / 1000,
        firstPaintTime: (base + Math.floor(Math.random() * 400)) / 1000,
        firstPaintAfterLoadTime: 0,
        navigationType: 'Other',
        wasFetchedViaSpdy: true,
        wasNpnNegotiated: true,
        npnNegotiatedProtocol: 'h2',
        wasAlternateProtocolAvailable: false,
        connectionInfo: 'h2'
      };
    }
  };
}

// === Screen ===
class Screen {
  constructor(w, h) {
    this._w = w;
    this._h = h;
    this.colorDepth = 24;
    this.pixelDepth = 24;
    this.availTop = 0;
    this.availLeft = 0;
    this.orientation = {
      type: 'landscape-primary',
      angle: 0,
      addEventListener: function() {},
      removeEventListener: function() {},
      dispatchEvent: function() { return true; }
    };
  }
  get width() { return this._w; }
  get height() { return this._h; }
  get availWidth() { return this._w; }
  get availHeight() { return this._h - 40; }
}

// === WebGL Renderer ===
function _getFp() {
  var uaPlatform = globalThis.__hpx_ua_platform || 'Windows';

  var windowsGpus = [
    { vendor: 'Google Inc. (NVIDIA)', renderer: 'ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)' },
    { vendor: 'Google Inc. (NVIDIA)', renderer: 'ANGLE (NVIDIA, NVIDIA GeForce RTX 3070 Direct3D11 vs_5_0 ps_5_0, D3D11)' },
    { vendor: 'Google Inc. (NVIDIA)', renderer: 'ANGLE (NVIDIA, NVIDIA GeForce RTX 3080 Direct3D11 vs_5_0 ps_5_0, D3D11)' },
    { vendor: 'Google Inc. (Intel)', renderer: 'ANGLE (Intel, Intel(R) UHD Graphics 630 Direct3D11 vs_5_0 ps_5_0, D3D11)' }
  ];

  var macGpus = [
    { vendor: 'Google Inc. (Apple)', renderer: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M1, Unspecified Version)' },
    { vendor: 'Google Inc. (Apple)', renderer: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)' },
    { vendor: 'Google Inc. (Apple)', renderer: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M3, Unspecified Version)' },
    { vendor: 'Google Inc. (Apple)', renderer: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M3 Pro, Unspecified Version)' }
  ];

  var linuxGpus = [
    { vendor: 'Google Inc. (Intel)', renderer: 'ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)' },
    { vendor: 'Google Inc. (Mesa)', renderer: 'ANGLE (Mesa, llvmpipe (LLVM 15.0.7, 256 bits), OpenGL 4.5 (Compatibility Profile) Mesa 23.2.1-1ubuntu3.1)' },
    { vendor: 'Google Inc. (NVIDIA)', renderer: 'ANGLE (NVIDIA, NVIDIA GeForce GTX 1660 Ti/PCIe/SSE2, OpenGL 4.5)' }
  ];

  var pools = { Windows: windowsGpus, macOS: macGpus, Linux: linuxGpus };
  var pool = pools[uaPlatform] || windowsGpus;
  var idx = Math.floor(_fpRand(42) * pool.length);
  var gpu = pool[idx % pool.length];

  var screenPool = [
    { w: 1920, h: 1080 },
    { w: 2560, h: 1440 },
    { w: 1366, h: 768 },
    { w: 1536, h: 864 },
    { w: 1440, h: 900 },
    { w: 1280, h: 720 }
  ];
  var sIdx = Math.floor(_fpRand(99) * screenPool.length);
  var scr = screenPool[sIdx % screenPool.length];

  return {
    vendor: gpu.vendor,
    renderer: gpu.renderer,
    screenW: scr.w,
    screenH: scr.h
  };
}

// === Stub Objects ===
function _buildStubs() {
  var NotifStub = function(title, options) {
    this.title = title || '';
    this.body = (options && options.body) || '';
    this.icon = (options && options.icon) || '';
    this.tag = (options && options.tag) || '';
  };
  NotifStub.permission = 'default';
  NotifStub.requestPermission = function() { return Promise.resolve('default'); };
  NotifStub.maxActions = 2;

  function WebGLContextStub() {}
  WebGLContextStub.prototype = {
    canvas: null,
    drawingBufferWidth: 0,
    drawingBufferHeight: 0,
    getParameter: function() { return null; },
    getExtension: function() { return null; },
    getSupportedExtensions: function() { return []; },
    getShaderPrecisionFormat: function() {
      return { rangeMin: 127, rangeMax: 127, precision: 23 };
    },
    createBuffer: function() { return {}; },
    createTexture: function() { return {}; },
    createProgram: function() { return {}; },
    createShader: function() { return {}; },
    shaderSource: function() {},
    compileShader: function() {},
    attachShader: function() {},
    linkProgram: function() {},
    useProgram: function() {},
    getAttribLocation: function() { return 0; },
    getUniformLocation: function() { return {}; },
    enableVertexAttribArray: function() {},
    vertexAttribPointer: function() {},
    uniform1f: function() {},
    uniform2f: function() {},
    uniform4f: function() {},
    uniformMatrix4fv: function() {},
    bindBuffer: function() {},
    bufferData: function() {},
    bindTexture: function() {},
    texImage2D: function() {},
    texParameteri: function() {},
    activeTexture: function() {},
    viewport: function() {},
    clear: function() {},
    clearColor: function() {},
    enable: function() {},
    disable: function() {},
    blendFunc: function() {},
    drawArrays: function() {},
    drawElements: function() {},
    getError: function() { return 0; },
    isContextLost: function() { return false; }
  };

  function WebGL2ContextStub() {}
  WebGL2ContextStub.prototype = Object.create(WebGLContextStub.prototype);
  WebGL2ContextStub.prototype.constructor = WebGL2ContextStub;

  return {
    Notification: NotifStub,
    WebGLRenderingContext: WebGLContextStub,
    WebGL2RenderingContext: WebGL2ContextStub
  };
}

// === AudioContext Stub ===
function _buildAudioContextStub() {
  function AudioContext() {
    this.sampleRate = [44100, 48000][Math.floor(_fpRand(101) * 2)];
    this.baseLatency = 0.002 + _fpRand(100) * 0.008;
    this.state = 'running';
    this.destination = { numberOfInputs: 1, numberOfOutputs: 0 };
    this.suspended = function() { return this.state === 'suspended'; };
    this.running = function() { return this.state === 'running'; };
    this.close = function() { this.state = 'closed'; return Promise.resolve(); };
    this.resume = function() { this.state = 'running'; return Promise.resolve(); };
    this.suspend = function() { this.state = 'suspended'; return Promise.resolve(); };
    this.createOscillator = function() { return { connect: function(){}, disconnect: function(){}, start: function(){}, stop: function(){}, frequency: { value: 440, setValueAtTime: function(){} }, type: 'sine' }; };
    this.createGain = function() { return { connect: function(){}, disconnect: function(){}, gain: { value: 1, setValueAtTime: function(){}, linearRampToValueAtTime: function(){} } }; };
    this.createAnalyser = function() { return { connect: function(){}, disconnect: function(){}, fftSize: 2048, frequencyBinCount: 1024, getByteFrequencyData: function(){}, getByteTimeDomainData: function(){} }; };
    this.createBuffer = function(ch, len, sr) { return { numberOfChannels: ch, length: len, sampleRate: sr, getChannelData: function(c){ return new Float32Array(len); } }; };
    this.createBufferSource = function() { return { connect: function(){}, disconnect: function(){}, start: function(){}, stop: function(){}, buffer: null }; };
    this.createMediaStreamSource = function() { return { connect: function(){}, disconnect: function(){} }; };
    this.createMediaElementSource = function() { return { connect: function(){}, disconnect: function(){} }; };
  }

  globalThis.AudioContext = AudioContext;
  globalThis.webkitAudioContext = AudioContext;
}

// === Performance Stub ===
function _buildPerformanceStub() {
  var t0 = Date.now();
  var heapLimit = 4294705152;
  var totalHeap = Math.floor(heapLimit * (0.3 + _fpRand(66) * 0.4));
  var usedHeap = Math.floor(totalHeap * (0.5 + _fpRand(77) * 0.3));

  globalThis.performance = {
    timing: {
      navigationStart: t0,
      domContentLoadedEventEnd: t0,
      loadEventEnd: t0
    },
    memory: {
      jsHeapSizeLimit: heapLimit,
      totalJSHeapSize: totalHeap,
      usedJSHeapSize: usedHeap
    },
    timeOrigin: t0,
    now: function() { return Date.now() - t0; },
    getEntries: function() { return []; },
    getEntriesByName: function() { return []; },
    getEntriesByType: function() { return []; },
    mark: function() {},
    measure: function() {},
    clearMarks: function() {},
    clearMeasures: function() {},
    toJSON: function() { return {}; }
  };
}

// === Canvas Noise ===
function _installCanvasNoise() {
  try {
    if (typeof HTMLCanvasElement !== 'undefined') {
      var origToDataURL = HTMLCanvasElement.prototype.toDataURL;
      var origToBlob = HTMLCanvasElement.prototype.toBlob;

      HTMLCanvasElement.prototype.toDataURL = function() {
        try {
          var ctx = this.getContext('2d');
          if (ctx && this.width > 0 && this.height > 0) {
            var imageData = ctx.getImageData(0, 0, this.width, this.height);
            var data = imageData.data;
            for (var i = 0; i < data.length; i += 4) {
              var px = (i / 4) % this.width;
              var py = Math.floor((i / 4) / this.width);
              data[i] = Math.max(0, Math.min(255, data[i] + Math.round(_fpNoise(px, py, 0))));
              data[i + 1] = Math.max(0, Math.min(255, data[i + 1] + Math.round(_fpNoise(px, py, 1))));
              data[i + 2] = Math.max(0, Math.min(255, data[i + 2] + Math.round(_fpNoise(px, py, 2))));
            }
            ctx.putImageData(imageData, 0, 0);
          }
        } catch (e) {}
        return origToDataURL.apply(this, arguments);
      };

      HTMLCanvasElement.prototype.toBlob = function(callback) {
        try {
          var ctx = this.getContext('2d');
          if (ctx && this.width > 0 && this.height > 0) {
            var imageData = ctx.getImageData(0, 0, this.width, this.height);
            var data = imageData.data;
            for (var i = 0; i < data.length; i += 4) {
              var px = (i / 4) % this.width;
              var py = Math.floor((i / 4) / this.width);
              data[i] = Math.max(0, Math.min(255, data[i] + Math.round(_fpNoise(px, py, 0))));
              data[i + 1] = Math.max(0, Math.min(255, data[i + 1] + Math.round(_fpNoise(px, py, 1))));
              data[i + 2] = Math.max(0, Math.min(255, data[i + 2] + Math.round(_fpNoise(px, py, 2))));
            }
            ctx.putImageData(imageData, 0, 0);
          }
        } catch (e) {}
        return origToBlob.apply(this, arguments);
      };
    }
  } catch (e) {}
}

// === Page Init ===
var __hpx_init = function() {
  var stealth = globalThis.__hpx_stealth;

  _fpSeed = Date.now() ^ (Math.floor(Math.random() * 0xFFFFFFFF) >>> 0);
  _fpCache = null;

  var fp = _getFp();

  var hwValues = globalThis.__hpx_stealth ? [4, 6, 8, 12, 16] : [2, 4, 6, 8, 12, 16];
  var memValues = globalThis.__hpx_stealth ? [4, 8] : [0.25, 0.5, 1, 2, 4, 8];
  var hwIdx = Math.floor(_fpRand(33) * hwValues.length);
  var memIdx = Math.floor(_fpRand(44) * memValues.length);

  var ua = globalThis.__hpx_ua || 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
  var platform = globalThis.__hpx_platform || 'Win32';

  var pluginData = [
    { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
    { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
    { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' }
  ];

  var mimeData = [
    { type: 'application/pdf', suffixes: 'pdf', description: 'Portable Document Format', enabledPlugin: null },
    { type: 'application/x-google-chrome-pdf', suffixes: 'pdf', description: 'Portable Document Format', enabledPlugin: null },
    { type: 'application/x-nacl', suffixes: '', description: 'Native Client Executable', enabledPlugin: null }
  ];

  var plugins = { length: pluginData.length };
  pluginData.forEach(function(p, i) {
    plugins[i] = p;
    plugins[p.name] = p;
  });

  var mimeTypes = { length: mimeData.length };
  mimeData.forEach(function(m, i) {
    mimeTypes[i] = m;
    mimeTypes[m.type] = m;
  });

  globalThis.navigator = {
    get userAgent() { return ua; },
    get appVersion() { return ua.replace('Mozilla/', ''); },
    platform: platform,
    vendor: 'Google Inc.',
    vendorSub: '',
    productSub: '20030107',
    product: 'Gecko',
    appName: 'Netscape',
    appCodeName: 'Mozilla',
    language: 'en-US',
    languages: ['en-US', 'en'],
    cookieEnabled: true,
    doNotTrack: null,
    maxTouchPoints: 0,
    hardwareConcurrency: hwValues[hwIdx % hwValues.length],
    deviceMemory: memValues[memIdx % memValues.length],
    pdfViewerEnabled: true,
    onLine: true,
    get plugins() { return new PluginArray(pluginData.map(function(p) { return new Plugin(p.name, p.description, p.filename); })); },
    get mimeTypes() { return new MimeTypeArray(mimeData.map(function(m) { return new MimeType(m.type, m.suffixes, m.description); })); },
    mediaDevices: {
      enumerateDevices: function() { return Promise.resolve([]); },
      getUserMedia: function() { return Promise.reject(new DOMException('Not allowed', 'NotAllowedError')); },
      ondevicechange: null,
      addEventListener: function() {},
      removeEventListener: function() {},
      dispatchEvent: function() { return true; }
    },
    permissions: {
      query: function(params) {
        var n = params && params.name;
        if (n === 'notifications') return Promise.resolve({ state: (globalThis.Notification && Notification.permission === 'granted') ? 'granted' : 'prompt', onchange: null });
        if (n === 'geolocation' || n === 'camera' || n === 'microphone' || n === 'midi') return Promise.resolve({ state: 'prompt', onchange: null });
        return Promise.resolve({ state: 'granted', onchange: null });
      }
    },
    getBattery: function() {
      return Promise.resolve({
        charging: _fp('batteryCharging') > 0.5,
        chargingTime: 0,
        dischargingTime: Infinity,
        level: _fp('batteryLevel') * 0.5 + 0.5,
        addEventListener: function() {},
        removeEventListener: function() {}
      });
    },
    geolocation: {
      getCurrentPosition: function(success) {
        var lat = 50.1109 + (_fp('geoLat') - 0.5) * 0.02;
        var lon = 8.6821 + (_fp('geoLon') - 0.5) * 0.02;
        success({ coords: { latitude: lat, longitude: lon, accuracy: 10 + _fp('geoAcc') * 90, altitude: null, altitudeAccuracy: null, heading: null, speed: null }, timestamp: Date.now() });
      },
      watchPosition: function() { return 0; },
      clearWatch: function() {},
      addEventListener: function() {},
      removeEventListener: function() {}
    },
    connection: (typeof NetworkInformation !== 'undefined') ? new NetworkInformation() : {
      effectiveType: '4g',
      rtt: 50,
      downlink: 10.0,
      saveData: false,
      addEventListener: function() {},
      removeEventListener: function() {}
    },
    storage: {
      estimate: function() { return Promise.resolve({ quota: 0, usage: 0 }); },
      persist: function() { return Promise.resolve(false); },
      persisted: function() { return Promise.resolve(false); }
    },
    clipboard: {
      readText: function() { return Promise.resolve(''); },
      writeText: function() { return Promise.resolve(); },
      read: function() { return Promise.resolve(new ClipboardItem([])); },
      write: function() { return Promise.resolve(); }
    },
    serviceWorker: {
      ready: Promise.resolve(null),
      register: function() { return Promise.reject(new DOMException('Not supported', 'NotSupportedError')); },
      addEventListener: function() {},
      removeEventListener: function() {}
    },
    getGamepads: function() { return []; },
    sendBeacon: function() { return true; },
    javaEnabled: function() { return false; },
    createObjectURL: function() { return ''; },
    revokeObjectURL: function() {},
    registerProtocolHandler: function() {},
    addEventListener: function() {},
    removeEventListener: function() {},
    dispatchEvent: function() { return true; },
    toString: function() { return '[object Navigator]'; }
  };

  var navProto = Object.getPrototypeOf(globalThis.navigator);
  Object.defineProperty(navProto, 'webdriver', {
    get: function() { return false; },
    enumerable: true,
    configurable: true
  });

  var chrome = _buildChrome();
  globalThis.chrome = chrome;

  var stubs = _buildStubs();
  globalThis.Notification = stubs.Notification;
  globalThis.WebGLRenderingContext = stubs.WebGLRenderingContext;
  globalThis.WebGL2RenderingContext = stubs.WebGL2RenderingContext;

  globalThis.screen = new Screen(fp.screenW, fp.screenH);
  Object.defineProperty(globalThis, 'innerWidth', { value: fp.screenW, writable: false, configurable: true });
  Object.defineProperty(globalThis, 'innerHeight', { value: fp.screenH - 111, writable: false, configurable: true });
  Object.defineProperty(globalThis, 'outerWidth', { value: fp.screenW, writable: false, configurable: true });
  Object.defineProperty(globalThis, 'outerHeight', { value: fp.screenH, writable: false, configurable: true });
  Object.defineProperty(globalThis, 'devicePixelRatio', { value: 1.0, writable: false, configurable: true });
  Object.defineProperty(globalThis, 'screenX', { value: 0, writable: false, configurable: true });
  Object.defineProperty(globalThis, 'screenY', { value: 0, writable: false, configurable: true });

  var uaData = _buildUserAgentData();
  globalThis.navigator.userAgentData = uaData;
  globalThis.userAgentData = uaData;

  globalThis._greaseHeader = _greaseHeader;

  _installCanvasNoise();
  _buildAudioContextStub();
  _buildPerformanceStub();

  delete globalThis.__hpx_init;
  delete globalThis.__hpx_ua;
  delete globalThis.__hpx_platform;
  delete globalThis.__hpx_ua_platform;
  delete globalThis.__hpx_ua_platform_version;
  delete globalThis.__hpx_stealth;
}
