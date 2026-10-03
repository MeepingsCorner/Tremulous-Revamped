/*
 * trem_audio.c - the port's FMOD + Steam Audio sound path (Windows).
 *
 * The same design as godot-steamaudio-dotnet (Steam Audio driving FMOD's
 * spatializer DSPs with the game's simulation), in C against the real
 * headers so the structure layouts cannot drift, and without .NET:
 *
 *   FMOD Core plays the game's .wav files from memory. Every voice has a
 *   "Steam Audio Spatializer" DSP (HRTF binaural rendering, occlusion and
 *   transmission through the map geometry, air absorption) fed by its own
 *   IPLSource, and a listener-centric "Steam Audio Reverb" DSP on the master
 *   group renders the map's simulated reflections (convolution reverb).
 *   Direct simulation runs every frame on the game thread; reflections run
 *   on a worker thread every ~100 ms.
 *
 * The DLLs (fmod.dll, phonon.dll, phonon_fmod.dll) are loaded from the
 * directory the game passes to ta_init; fmod.dll and phonon.dll are linked
 * delay-loaded, so a missing DLL only means ta_init fails and the game keeps
 * Godot's audio.
 *
 * Coordinates: callers pass Godot space (x right, y up, z back, metres).
 * Steam Audio uses that right-handed space directly; FMOD (initialised
 * left-handed, as phonon_fmod expects: it flips z itself) gets (x, y, -z).
 */
#include <windows.h>
#include <string.h>
#include <stdio.h>
#include <math.h>

#include "fmod.h"
#include "fmod_errors.h"
#include "phonon.h"

#define TA_VOICES 64
#define TA_SOUNDS 4096

/* phonon_fmod.dll's exports (steamaudio_fmod.h) */
typedef void (*PFN_iplFMODInitialize)(IPLContext);
typedef void (*PFN_iplFMODTerminate)(void);
typedef void (*PFN_iplFMODSetHRTF)(IPLHRTF);
typedef void (*PFN_iplFMODSetSimulationSettings)(IPLSimulationSettings);
typedef void (*PFN_iplFMODSetReverbSource)(IPLSource);
typedef IPLint32 (*PFN_iplFMODAddSource)(IPLSource);
typedef void (*PFN_iplFMODRemoveSource)(IPLint32);

/* IPLSpatializerParams (api-reference: DSP parameters, in order) */
enum {
    SPAT_SOURCE_POSITION = 0,
    SPAT_OVERALL_GAIN,
    SPAT_APPLY_DISTANCEATTENUATION,
    SPAT_APPLY_AIRABSORPTION,
    SPAT_APPLY_DIRECTIVITY,
    SPAT_APPLY_OCCLUSION,
    SPAT_APPLY_TRANSMISSION,
    SPAT_APPLY_REFLECTIONS,
    SPAT_APPLY_PATHING,
    SPAT_HRTF_INTERPOLATION,
    SPAT_DISTANCEATTENUATION,
    SPAT_DISTANCEATTENUATION_ROLLOFFTYPE,
    SPAT_DISTANCEATTENUATION_MINDISTANCE,
    SPAT_DISTANCEATTENUATION_MAXDISTANCE,
    SPAT_AIRABSORPTION_LOW,
    SPAT_AIRABSORPTION_MID,
    SPAT_AIRABSORPTION_HIGH,
    SPAT_DIRECTIVITY,
    SPAT_DIRECTIVITY_DIPOLEWEIGHT,
    SPAT_DIRECTIVITY_DIPOLEPOWER,
    SPAT_OCCLUSION,
    SPAT_TRANSMISSION_TYPE,
    SPAT_TRANSMISSION_LOW,
    SPAT_TRANSMISSION_MID,
    SPAT_TRANSMISSION_HIGH,
    SPAT_DIRECT_MIXLEVEL,
    SPAT_REFLECTIONS_BINAURAL,
    SPAT_REFLECTIONS_MIXLEVEL,
    SPAT_PATHING_BINAURAL,
    SPAT_PATHING_MIXLEVEL,
    SPAT_SIMULATION_OUTPUTS,
    SPAT_DIRECT_BINAURAL,
    SPAT_DISTANCE_ATTENUATION_RANGE,
    SPAT_SIMULATION_OUTPUTS_HANDLE,
    SPAT_OUTPUT_FORMAT,
    SPAT_NORMALIZE_PATHING_EQ
};
/* IPLReverbParams */
enum { REVERB_BINAURAL = 0, REVERB_OUTPUT_FORMAT };

typedef struct {
    FMOD_CHANNEL* ch;
    FMOD_DSP* dsp;
    IPLSource src;
    IPLint32 handle;
    float pos[3];
    int local;
    int active;
    unsigned gen;
} Voice;

static struct {
    int up;
    HMODULE m_fmod, m_phonon, m_plugin;
    FMOD_SYSTEM* sys;
    unsigned plugin;
    unsigned spat_plugin, reverb_plugin;
    FMOD_DSP* reverb_dsp;
    FMOD_SOUND* sounds[TA_SOUNDS];
    int nsounds;
    Voice v[TA_VOICES];
    int next_voice;
    IPLContext ctx;
    IPLHRTF hrtf;
    IPLSimulator sim;
    IPLSimulationSettings simset;
    IPLScene scene;
    IPLStaticMesh mesh;
    IPLSource reverb_src;
    IPLint32 reverb_handle;
    float listener[12]; /* pos, fwd, up, right (Godot space) */
    int occlusion, reflections;
    float master;
    CRITICAL_SECTION lock;
    HANDLE thread;
    volatile LONG quit;
    volatile LONG have_scene;
    PFN_iplFMODInitialize fInit;
    PFN_iplFMODTerminate fTerm;
    PFN_iplFMODSetHRTF fSetHRTF;
    PFN_iplFMODSetSimulationSettings fSetSim;
    PFN_iplFMODSetReverbSource fSetReverb;
    PFN_iplFMODAddSource fAdd;
    PFN_iplFMODRemoveSource fRemove;
} S;

static char g_err[512];

static int fail(const char* what, const char* detail) {
    _snprintf(g_err, sizeof(g_err) - 1, "%s%s%s", what, detail ? ": " : "", detail ? detail : "");
    return 0;
}

__declspec(dllexport) const char* ta_error(void) { return g_err; }

static HMODULE load_from(const wchar_t* dir, const wchar_t* name) {
    wchar_t path[MAX_PATH * 2];
    _snwprintf(path, MAX_PATH * 2 - 1, L"%s\\%s", dir, name);
    path[MAX_PATH * 2 - 1] = 0;
    return LoadLibraryExW(path, NULL, LOAD_WITH_ALTERED_SEARCH_PATH);
}

static FMOD_VECTOR fv(const float* g) {
    FMOD_VECTOR v;
    v.x = g[0];
    v.y = g[1];
    v.z = -g[2];
    return v;
}
static IPLVector3 iv(const float* g) {
    IPLVector3 v;
    v.x = g[0];
    v.y = g[1];
    v.z = g[2];
    return v;
}

static DWORD WINAPI reflection_thread(LPVOID unused) {
    (void)unused;
    while (!S.quit) {
        Sleep(90);
        if (S.quit) break;
        if (!S.have_scene || !S.reflections) continue;
        EnterCriticalSection(&S.lock);
        iplSimulatorRunReflections(S.sim);
        LeaveCriticalSection(&S.lock);
    }
    return 0;
}

/* Find a nested plugin of phonon_fmod.dll by name. */
static unsigned find_plugin(const char* want) {
    int n = 0;
    if (FMOD_System_GetNumNestedPlugins(S.sys, S.plugin, &n) != FMOD_OK) return 0;
    for (int i = 0; i < n; i++) {
        unsigned h = 0;
        char name[128] = {0};
        FMOD_PLUGINTYPE type;
        unsigned ver = 0;
        if (FMOD_System_GetNestedPlugin(S.sys, S.plugin, i, &h) != FMOD_OK) continue;
        if (FMOD_System_GetPluginInfo(S.sys, h, &type, name, sizeof(name), &ver) != FMOD_OK) continue;
        if (strstr(name, want)) return h;
    }
    return 0;
}

/* Bring up FMOD + Steam Audio. `dir`: the folder with the three DLLs. */
__declspec(dllexport) int ta_init(const wchar_t* dir, int sample_rate) {
    FMOD_RESULT r;
    if (S.up) return 1;
    memset(&S, 0, sizeof(S));
    g_err[0] = 0;
    /* phonon_fmod.dll depends on phonon.dll: load it first, by full path */
    S.m_phonon = load_from(dir, L"phonon.dll");
    if (!S.m_phonon) return fail("phonon.dll not found", NULL);
    S.m_fmod = load_from(dir, L"fmod.dll");
    if (!S.m_fmod) return fail("fmod.dll not found", NULL);
    S.m_plugin = load_from(dir, L"phonon_fmod.dll");
    if (!S.m_plugin) return fail("phonon_fmod.dll not found", NULL);
    S.fInit = (PFN_iplFMODInitialize)GetProcAddress(S.m_plugin, "iplFMODInitialize");
    S.fTerm = (PFN_iplFMODTerminate)GetProcAddress(S.m_plugin, "iplFMODTerminate");
    S.fSetHRTF = (PFN_iplFMODSetHRTF)GetProcAddress(S.m_plugin, "iplFMODSetHRTF");
    S.fSetSim = (PFN_iplFMODSetSimulationSettings)GetProcAddress(S.m_plugin, "iplFMODSetSimulationSettings");
    S.fSetReverb = (PFN_iplFMODSetReverbSource)GetProcAddress(S.m_plugin, "iplFMODSetReverbSource");
    S.fAdd = (PFN_iplFMODAddSource)GetProcAddress(S.m_plugin, "iplFMODAddSource");
    S.fRemove = (PFN_iplFMODRemoveSource)GetProcAddress(S.m_plugin, "iplFMODRemoveSource");
    if (!S.fInit || !S.fSetHRTF || !S.fSetSim || !S.fAdd || !S.fRemove || !S.fSetReverb) return fail("phonon_fmod.dll lacks the iplFMOD* functions", NULL);

    /* FMOD */
    if ((r = FMOD_System_Create(&S.sys, FMOD_VERSION)) != FMOD_OK) return fail("FMOD_System_Create", FMOD_ErrorString(r));
    FMOD_System_SetSoftwareFormat(S.sys, sample_rate, FMOD_SPEAKERMODE_STEREO, 0);
    FMOD_System_SetDSPBufferSize(S.sys, 1024, 4);
    if ((r = FMOD_System_Init(S.sys, 256, FMOD_INIT_NORMAL, NULL)) != FMOD_OK) return fail("FMOD_System_Init", FMOD_ErrorString(r));
    FMOD_System_Set3DSettings(S.sys, 1.0f, 1.0f, 1.0f);

    /* Steam Audio */
    IPLContextSettings cs;
    memset(&cs, 0, sizeof(cs));
    cs.version = STEAMAUDIO_VERSION;
    if (iplContextCreate(&cs, &S.ctx) != IPL_STATUS_SUCCESS) return fail("iplContextCreate", NULL);
    IPLAudioSettings as;
    as.samplingRate = sample_rate;
    as.frameSize = 1024;
    IPLHRTFSettings hs;
    memset(&hs, 0, sizeof(hs));
    hs.type = IPL_HRTFTYPE_DEFAULT;
    hs.volume = 1.0f;
    hs.normType = IPL_HRTFNORMTYPE_NONE;
    if (iplHRTFCreate(S.ctx, &as, &hs, &S.hrtf) != IPL_STATUS_SUCCESS) return fail("iplHRTFCreate", NULL);

    memset(&S.simset, 0, sizeof(S.simset));
    S.simset.flags = (IPLSimulationFlags)(IPL_SIMULATIONFLAGS_DIRECT | IPL_SIMULATIONFLAGS_REFLECTIONS);
    S.simset.sceneType = IPL_SCENETYPE_DEFAULT;
    S.simset.reflectionType = IPL_REFLECTIONEFFECTTYPE_CONVOLUTION;
    S.simset.maxNumOcclusionSamples = 16;
    S.simset.maxNumRays = 2048;
    S.simset.numDiffuseSamples = 32;
    S.simset.maxDuration = 1.5f;
    S.simset.maxOrder = 1;
    S.simset.maxNumSources = TA_VOICES + 2;
    S.simset.numThreads = 2;
    S.simset.rayBatchSize = 1;
    S.simset.numVisSamples = 0;
    S.simset.samplingRate = sample_rate;
    S.simset.frameSize = 1024;
    if (iplSimulatorCreate(S.ctx, &S.simset, &S.sim) != IPL_STATUS_SUCCESS) return fail("iplSimulatorCreate", NULL);

    S.fInit(S.ctx);
    S.fSetHRTF(S.hrtf);
    S.fSetSim(S.simset);

    /* the plugin's effects */
    if ((r = FMOD_System_LoadPlugin(S.sys, "phonon_fmod.dll", &S.plugin, 0)) != FMOD_OK) {
        /* by full path when the search path misses it */
        char path[MAX_PATH * 2];
        WideCharToMultiByte(CP_UTF8, 0, dir, -1, path, sizeof(path) - 32, NULL, NULL);
        strcat(path, "\\phonon_fmod.dll");
        if ((r = FMOD_System_LoadPlugin(S.sys, path, &S.plugin, 0)) != FMOD_OK) return fail("FMOD_System_LoadPlugin phonon_fmod", FMOD_ErrorString(r));
    }
    S.spat_plugin = find_plugin("Spatializer");
    S.reverb_plugin = find_plugin("Reverb");
    if (!S.spat_plugin) return fail("no Steam Audio Spatializer in phonon_fmod.dll", NULL);

    /* the listener-centric reverb */
    IPLSourceSettings ss;
    ss.flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
    if (iplSourceCreate(S.sim, &ss, &S.reverb_src) == IPL_STATUS_SUCCESS) {
        iplSourceAdd(S.reverb_src, S.sim);
        S.fSetReverb(S.reverb_src);
        if (S.reverb_plugin && FMOD_System_CreateDSPByPlugin(S.sys, S.reverb_plugin, &S.reverb_dsp) == FMOD_OK) {
            FMOD_CHANNELGROUP* master = NULL;
            FMOD_System_GetMasterChannelGroup(S.sys, &master);
            FMOD_DSP_SetParameterBool(S.reverb_dsp, REVERB_BINAURAL, 1);
            FMOD_ChannelGroup_AddDSP(master, FMOD_CHANNELCONTROL_DSP_TAIL, S.reverb_dsp);
        }
    }

    /* the voices: one spatializer + one simulation source each */
    for (int i = 0; i < TA_VOICES; i++) {
        Voice* v = &S.v[i];
        ss.flags = IPL_SIMULATIONFLAGS_DIRECT;
        if (iplSourceCreate(S.sim, &ss, &v->src) != IPL_STATUS_SUCCESS) return fail("iplSourceCreate", NULL);
        iplSourceAdd(v->src, S.sim);
        v->handle = S.fAdd(v->src);
        if (FMOD_System_CreateDSPByPlugin(S.sys, S.spat_plugin, &v->dsp) != FMOD_OK) return fail("FMOD_System_CreateDSPByPlugin spatializer", NULL);
        FMOD_DSP_SetParameterInt(v->dsp, SPAT_SIMULATION_OUTPUTS_HANDLE, v->handle);
        FMOD_DSP_SetParameterInt(v->dsp, SPAT_APPLY_DISTANCEATTENUATION, 1);
        FMOD_DSP_SetParameterInt(v->dsp, SPAT_APPLY_AIRABSORPTION, 1);
        FMOD_DSP_SetParameterInt(v->dsp, SPAT_APPLY_OCCLUSION, 1);
        FMOD_DSP_SetParameterInt(v->dsp, SPAT_APPLY_TRANSMISSION, 1);
        FMOD_DSP_SetParameterBool(v->dsp, SPAT_APPLY_REFLECTIONS, 0);
        FMOD_DSP_SetParameterBool(v->dsp, SPAT_DIRECT_BINAURAL, 1);
        FMOD_DSP_SetParameterInt(v->dsp, SPAT_HRTF_INTERPOLATION, 1);
    }
    iplSimulatorCommit(S.sim);

    S.occlusion = 1;
    S.reflections = 1;
    S.master = 1.0f;
    S.listener[5] = -1.0f; /* forward -z */
    S.listener[7] = 1.0f;  /* up +y */
    S.listener[9] = 1.0f;  /* right +x */
    InitializeCriticalSection(&S.lock);
    S.thread = CreateThread(NULL, 0, reflection_thread, NULL, 0, NULL);
    S.up = 1;
    return 1;
}

__declspec(dllexport) void ta_shutdown(void) {
    if (!S.up) return;
    S.quit = 1;
    if (S.thread) {
        WaitForSingleObject(S.thread, 2000);
        CloseHandle(S.thread);
    }
    for (int i = 0; i < TA_VOICES; i++) {
        Voice* v = &S.v[i];
        if (v->ch) FMOD_Channel_Stop(v->ch);
        if (v->dsp) FMOD_DSP_Release(v->dsp);
        if (v->handle) S.fRemove(v->handle);
        if (v->src) {
            iplSourceRemove(v->src, S.sim);
            iplSourceRelease(&v->src);
        }
    }
    if (S.reverb_dsp) FMOD_DSP_Release(S.reverb_dsp);
    for (int i = 0; i < S.nsounds; i++)
        if (S.sounds[i]) FMOD_Sound_Release(S.sounds[i]);
    FMOD_System_Release(S.sys);
    if (S.fTerm) S.fTerm();
    if (S.reverb_src) iplSourceRelease(&S.reverb_src);
    if (S.mesh) iplStaticMeshRelease(&S.mesh);
    if (S.scene) iplSceneRelease(&S.scene);
    if (S.sim) iplSimulatorRelease(&S.sim);
    if (S.hrtf) iplHRTFRelease(&S.hrtf);
    if (S.ctx) iplContextRelease(&S.ctx);
    DeleteCriticalSection(&S.lock);
    S.up = 0;
}

/* The map's geometry for occlusion, transmission and reflections (Godot
 * space). `mat`: per-triangle material index into the table below. */
__declspec(dllexport) int ta_set_scene(const float* verts, int nverts, const int* tris, int ntris, const int* mat) {
    static IPLMaterial materials[3] = {
        /* generic / concrete-ish */
        {{0.10f, 0.20f, 0.30f}, 0.05f, {0.100f, 0.050f, 0.030f}},
        /* metal (human base) */
        {{0.20f, 0.07f, 0.06f}, 0.05f, {0.200f, 0.025f, 0.010f}},
        /* rock / organic */
        {{0.13f, 0.20f, 0.24f}, 0.10f, {0.015f, 0.002f, 0.001f}},
    };
    if (!S.up) return 0;
    EnterCriticalSection(&S.lock);
    InterlockedExchange(&S.have_scene, 0);
    if (S.mesh) {
        iplStaticMeshRemove(S.mesh, S.scene);
        iplStaticMeshRelease(&S.mesh);
    }
    if (S.scene) iplSceneRelease(&S.scene);
    IPLSceneSettings sc;
    memset(&sc, 0, sizeof(sc));
    sc.type = IPL_SCENETYPE_DEFAULT;
    int ok = iplSceneCreate(S.ctx, &sc, &S.scene) == IPL_STATUS_SUCCESS;
    if (ok && ntris > 0) {
        IPLStaticMeshSettings ms;
        ms.numVertices = nverts;
        ms.numTriangles = ntris;
        ms.numMaterials = 3;
        ms.vertices = (IPLVector3*)verts;
        ms.triangles = (IPLTriangle*)tris;
        ms.materialIndices = (IPLint32*)mat;
        ms.materials = materials;
        ok = iplStaticMeshCreate(S.scene, &ms, &S.mesh) == IPL_STATUS_SUCCESS;
        if (ok) {
            iplStaticMeshAdd(S.mesh, S.scene);
            iplSceneCommit(S.scene);
        }
    }
    if (ok) {
        iplSimulatorSetScene(S.sim, S.scene);
        iplSimulatorCommit(S.sim);
        InterlockedExchange(&S.have_scene, 1);
    }
    LeaveCriticalSection(&S.lock);
    if (!ok) fail("Steam Audio scene creation failed", NULL);
    return ok;
}

/* A sound from a .wav / .ogg file's bytes; -1 on failure. */
__declspec(dllexport) int ta_sound_load(const void* data, int len) {
    if (!S.up || S.nsounds >= TA_SOUNDS) return -1;
    FMOD_CREATESOUNDEXINFO ex;
    memset(&ex, 0, sizeof(ex));
    ex.cbsize = sizeof(ex);
    ex.length = (unsigned)len;
    FMOD_SOUND* s = NULL;
    FMOD_RESULT r = FMOD_System_CreateSound(S.sys, (const char*)data, FMOD_OPENMEMORY | FMOD_CREATESAMPLE | FMOD_2D | FMOD_LOOP_OFF, &ex, &s);
    if (r != FMOD_OK) {
        fail("FMOD_System_CreateSound", FMOD_ErrorString(r));
        return -1;
    }
    S.sounds[S.nsounds] = s;
    return S.nsounds++;
}

static int voice_playing(Voice* v) {
    FMOD_BOOL p = 0;
    if (!v->ch) return 0;
    if (FMOD_Channel_IsPlaying(v->ch, &p) != FMOD_OK) return 0;
    return p ? 1 : 0;
}

static void set_source_position(Voice* v) {
    FMOD_DSP_PARAMETER_3DATTRIBUTES a;
    memset(&a, 0, sizeof(a));
    a.absolute.position = fv(v->pos);
    float fwd[3] = {0.0f, 0.0f, -1.0f}, up[3] = {0.0f, 1.0f, 0.0f};
    a.absolute.forward = fv(fwd);
    a.absolute.up = fv(up);
    /* relative to the listener (FMOD's own convention, left-handed) */
    float d[3] = {v->pos[0] - S.listener[0], v->pos[1] - S.listener[1], v->pos[2] - S.listener[2]};
    float* lf = &S.listener[3];
    float* lu = &S.listener[6];
    float* lr = &S.listener[9];
    a.relative.position.x = d[0] * lr[0] + d[1] * lr[1] + d[2] * lr[2];
    a.relative.position.y = d[0] * lu[0] + d[1] * lu[1] + d[2] * lu[2];
    a.relative.position.z = d[0] * lf[0] + d[1] * lf[1] + d[2] * lf[2];
    a.relative.forward.z = 1.0f;
    a.relative.up.y = 1.0f;
    FMOD_DSP_SetParameterData(v->dsp, SPAT_SOURCE_POSITION, &a, sizeof(a));
}

/* Play sound `id`; `local`: the listener's own sound (no spatialization).
 * Returns a voice id (index | generation << 8) or -1. */
__declspec(dllexport) int ta_play(int id, const float* pos, int local, int loop, float volume) {
    if (!S.up || id < 0 || id >= S.nsounds) return -1;
    int best = -1;
    for (int k = 0; k < TA_VOICES; k++) {
        int i = (S.next_voice + k) % TA_VOICES;
        if (!voice_playing(&S.v[i])) {
            best = i;
            break;
        }
    }
    if (best < 0) {
        /* steal the oldest non-looping voice */
        best = S.next_voice;
        FMOD_Channel_Stop(S.v[best].ch);
    }
    S.next_voice = (best + 1) % TA_VOICES;
    Voice* v = &S.v[best];
    if (v->ch) {
        FMOD_Channel_RemoveDSP(v->ch, v->dsp);
    }
    /* a fresh start for the reused spatializer (no tail from the last sound) */
    FMOD_DSP_DisconnectAll(v->dsp, 1, 1);
    FMOD_DSP_Reset(v->dsp);
    FMOD_CHANNEL* ch = NULL;
    if (FMOD_System_PlaySound(S.sys, S.sounds[id], NULL, 1, &ch) != FMOD_OK) return -1;
    FMOD_Channel_SetMode(ch, loop ? FMOD_LOOP_NORMAL : FMOD_LOOP_OFF);
    FMOD_Channel_SetVolume(ch, volume);
    v->ch = ch;
    v->local = local;
    v->active = 1;
    v->gen = (v->gen + 1) & 0x7fffff;
    if (pos) memcpy(v->pos, pos, sizeof(v->pos));
    if (!local) {
        set_source_position(v);
        FMOD_Channel_AddDSP(ch, 0, v->dsp);
    }
    FMOD_Channel_SetPaused(ch, 0);
    return best | (int)(v->gen << 8);
}

static Voice* voice_of(int h) {
    if (h < 0) return NULL;
    Voice* v = &S.v[h & 0xff];
    if ((h & 0xff) >= TA_VOICES || v->gen != (unsigned)(h >> 8)) return NULL;
    return v;
}

__declspec(dllexport) void ta_voice_set_pos(int h, const float* pos) {
    Voice* v = voice_of(h);
    if (!S.up || !v) return;
    memcpy(v->pos, pos, sizeof(v->pos));
}

__declspec(dllexport) void ta_voice_stop(int h) {
    Voice* v = voice_of(h);
    if (!S.up || !v || !v->ch) return;
    FMOD_Channel_Stop(v->ch);
    v->active = 0;
}

__declspec(dllexport) int ta_voice_playing(int h) {
    Voice* v = voice_of(h);
    return (S.up && v) ? voice_playing(v) : 0;
}

/* The listener (Godot space): position, forward, up. */
__declspec(dllexport) void ta_listener(const float* pos, const float* fwd, const float* up) {
    if (!S.up) return;
    memcpy(&S.listener[0], pos, 12);
    memcpy(&S.listener[3], fwd, 12);
    memcpy(&S.listener[6], up, 12);
    /* right = fwd x up */
    S.listener[9] = fwd[1] * up[2] - fwd[2] * up[1];
    S.listener[10] = fwd[2] * up[0] - fwd[0] * up[2];
    S.listener[11] = fwd[0] * up[1] - fwd[1] * up[0];
    FMOD_VECTOR p = fv(pos), f = fv(fwd), u = fv(up), vel = {0.0f, 0.0f, 0.0f};
    FMOD_System_Set3DListenerAttributes(S.sys, 0, &p, &vel, &f, &u);
}

__declspec(dllexport) void ta_options(int occlusion, int reflections, float master) {
    if (!S.up) return;
    S.occlusion = occlusion;
    S.reflections = reflections;
    S.master = master;
    FMOD_CHANNELGROUP* mg = NULL;
    if (FMOD_System_GetMasterChannelGroup(S.sys, &mg) == FMOD_OK) FMOD_ChannelGroup_SetVolume(mg, master);
    for (int i = 0; i < TA_VOICES; i++) {
        if (!S.v[i].dsp) continue;
        FMOD_DSP_SetParameterInt(S.v[i].dsp, SPAT_APPLY_OCCLUSION, occlusion ? 1 : 0);
        FMOD_DSP_SetParameterInt(S.v[i].dsp, SPAT_APPLY_TRANSMISSION, occlusion ? 1 : 0);
    }
    if (S.reverb_dsp) FMOD_DSP_SetBypass(S.reverb_dsp, reflections ? 0 : 1);
}

/* Once per frame: the simulation inputs, the direct simulation, FMOD. */
__declspec(dllexport) void ta_update(void) {
    if (!S.up) return;
    for (int i = 0; i < TA_VOICES; i++) {
        Voice* v = &S.v[i];
        if (v->active && !v->local && voice_playing(v)) set_source_position(v);
    }
    if (S.have_scene && TryEnterCriticalSection(&S.lock)) {
        IPLSimulationSharedInputs sh;
        memset(&sh, 0, sizeof(sh));
        sh.listener.origin = iv(&S.listener[0]);
        sh.listener.ahead = iv(&S.listener[3]);
        sh.listener.up = iv(&S.listener[6]);
        sh.listener.right = iv(&S.listener[9]);
        sh.numRays = 1024;
        sh.numBounces = 8;
        sh.duration = 1.5f;
        sh.order = 1;
        sh.irradianceMinDistance = 1.0f;
        iplSimulatorSetSharedInputs(S.sim, (IPLSimulationFlags)(IPL_SIMULATIONFLAGS_DIRECT | IPL_SIMULATIONFLAGS_REFLECTIONS), &sh);
        for (int i = 0; i < TA_VOICES; i++) {
            Voice* v = &S.v[i];
            if (!v->active || v->local) continue;
            IPLSimulationInputs in;
            memset(&in, 0, sizeof(in));
            in.flags = IPL_SIMULATIONFLAGS_DIRECT;
            in.directFlags = (IPLDirectSimulationFlags)(IPL_DIRECTSIMULATIONFLAGS_OCCLUSION | IPL_DIRECTSIMULATIONFLAGS_TRANSMISSION);
            in.source.origin = iv(v->pos);
            in.source.ahead.z = -1.0f;
            in.source.up.y = 1.0f;
            in.source.right.x = 1.0f;
            in.distanceAttenuationModel.type = IPL_DISTANCEATTENUATIONTYPE_DEFAULT;
            in.airAbsorptionModel.type = IPL_AIRABSORPTIONTYPE_DEFAULT;
            in.occlusionType = IPL_OCCLUSIONTYPE_VOLUMETRIC;
            in.occlusionRadius = 0.6f;
            in.numOcclusionSamples = 12;
            in.numTransmissionRays = 2;
            iplSourceSetInputs(v->src, IPL_SIMULATIONFLAGS_DIRECT, &in);
        }
        if (S.reverb_src) {
            IPLSimulationInputs in;
            memset(&in, 0, sizeof(in));
            in.flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
            in.source = sh.listener;
            in.reverbScale[0] = in.reverbScale[1] = in.reverbScale[2] = 1.0f;
            in.distanceAttenuationModel.type = IPL_DISTANCEATTENUATIONTYPE_DEFAULT;
            in.airAbsorptionModel.type = IPL_AIRABSORPTIONTYPE_DEFAULT;
            iplSourceSetInputs(S.reverb_src, IPL_SIMULATIONFLAGS_REFLECTIONS, &in);
        }
        if (S.occlusion) iplSimulatorRunDirect(S.sim);
        LeaveCriticalSection(&S.lock);
    }
    for (int i = 0; i < TA_VOICES; i++) {
        if (S.v[i].active && !voice_playing(&S.v[i])) S.v[i].active = 0;
    }
    FMOD_System_Update(S.sys);
}
