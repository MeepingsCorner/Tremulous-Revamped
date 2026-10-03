/* Network oracle (dec033): the ORIGINAL qcommon/msg.c, huffman.c and
 * net_chan.c compiled natively. Emits, as hex lines, the bytes of a
 * deterministic sequence of MSG_WriteBits / strings / keyed usercmd deltas /
 * entityState and playerState deltas / Huff_Compress / Netchan_Transmit
 * packets. crates/trem_sim/tests/net.rs produces the same sequence with the
 * Rust port and compares the SHA-256 with re/traces/net.sha256.
 * usage: net_ref | sha256sum */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include "q_shared.h"
#include "qcommon.h"

/* ---- engine shims -------------------------------------------------- */
cvar_t *cl_shownet = NULL;
cvar_t *com_timescale, *cl_packetdelay, *sv_packetdelay;
static cvar_t cvars[16];
static int ncvars;
cvar_t *Cvar_Get(const char *name, const char *value, int flags) {
  cvar_t *c = &cvars[ncvars++];
  (void)flags;
  c->name = (char *)name;
  c->string = (char *)value;
  c->integer = atoi(value);
  c->value = (float)atof(value);
  return c;
}
void QDECL Com_Printf(const char *fmt, ...) { (void)fmt; }
void QDECL Com_DPrintf(const char *fmt, ...) { (void)fmt; }
void QDECL Com_Error(int level, const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); vfprintf(stderr, fmt, ap); va_end(ap);
  (void)level; exit(2);
}
int Sys_Milliseconds(void) { return 0; }
void *S_MallocDebug(int size, char *l, char *f, int ln) { (void)l; (void)f; (void)ln; return malloc(size); }
void Z_Free(void *p) { free(p); }
const char *NET_AdrToString(netadr_t a) { (void)a; return "x"; }
qboolean Sys_StringToAdr(const char *s, netadr_t *a, netadrtype_t f) { (void)s; (void)a; (void)f; return qfalse; }
static void hex(const char *tag, const byte *d, int n) {
  printf("%s %d ", tag, n);
  for (int i = 0; i < n; i++) printf("%02x", d[i]);
  printf("\n");
}
void Sys_SendPacket(int length, const void *data, netadr_t to) {
  (void)to;
  hex("pkt", (const byte *)data, length);
}

/* ---- deterministic generator shared with tests/net.rs --------------- */
static unsigned int seed = 12345u;
static unsigned int lcg(void) { seed = seed * 1103515245u + 12345u; return seed >> 8; }
/* two draws, explicitly sequenced (operand order is unspecified in C) */
static unsigned int lcg2(void) { unsigned int a = lcg(); unsigned int b = lcg(); return a ^ (b << 8); }

static int gen_word(int bits) {
  unsigned int r = lcg() % 10;
  if (r < 4) return 0;
  if (bits == 0) {
    float f;
    floatint_t fi;
    if (r < 7) f = (float)((int)(lcg() % 8000) - 4000);
    else f = (float)((int)(lcg() % 2000000) - 1000000) / 64.0f;
    fi.f = f;
    return fi.i;
  }
  {
    int ab = bits < 0 ? -bits : bits;
    unsigned int v = (r < 7) ? lcg() % 256 : lcg2();
    if (ab < 32) v &= (1u << ab) - 1;
    return (int)v;
  }
}

/* msg.c's netField_t (file-local typedef there) */
typedef struct { char *name; int offset; int bits; } netField_t;
extern netField_t entityStateFields[];
extern netField_t playerStateFields[];

static void gen_es(entityState_t *es) {
  int i;
  memset(es, 0, sizeof(*es));
  es->number = lcg() % (MAX_GENTITIES - 1);
  for (i = 0; i < 52; i++)
    *(int *)((byte *)es + entityStateFields[i].offset) = gen_word(entityStateFields[i].bits);
}
static void gen_ps(playerState_t *ps) {
  int i;
  memset(ps, 0, sizeof(*ps));
  for (i = 0; i < 52; i++)
    *(int *)((byte *)ps + playerStateFields[i].offset) = gen_word(playerStateFields[i].bits);
  for (i = 0; i < MAX_STATS; i++) ps->stats[i] = (short)gen_word(16);
  for (i = 0; i < MAX_PERSISTANT; i++) ps->persistant[i] = (short)gen_word(16);
  for (i = 0; i < MAX_MISC; i++) ps->misc[i] = gen_word(32);
}
static void gen_cmd(usercmd_t *c, int base) {
  c->serverTime = base + (int)(lcg() % 600);
  c->angles[0] = lcg() % 65536; c->angles[1] = lcg() % 65536; c->angles[2] = lcg() % 4 ? 0 : (int)(lcg() % 65536);
  c->buttons = lcg() % 4 ? 0 : (int)(lcg() % 65536);
  c->weapon = lcg() % 26;
  c->forwardmove = (signed char)(lcg() % 3 ? 127 : (int)(lcg() % 256) - 128);
  c->rightmove = (signed char)((int)(lcg() % 256) - 128);
  c->upmove = (signed char)(lcg() % 2 ? 0 : 127);
}

int main(void) {
  static byte buf[MAX_MSGLEN];
  msg_t m;
  int i, k;
  com_timescale = Cvar_Get("timescale", "1", 0);
  cl_packetdelay = Cvar_Get("cl_packetdelay", "0", 0);
  sv_packetdelay = Cvar_Get("sv_packetdelay", "0", 0);
  Netchan_Init(4242);

  /* 1. raw bits through the Huffman bitstream */
  memset(buf, 0, sizeof(buf)); /* MSG_WriteBits counts a byte it has not cleared yet */
    MSG_Init(&m, buf, sizeof(buf));
  for (i = 0; i < 3000; i++) {
    int bits = 1 + (int)(lcg() % 32);
    unsigned int v = lcg2();
    if (lcg() % 3 == 0 && bits < 32) MSG_WriteBits(&m, (int)v, -bits);
    else MSG_WriteBits(&m, (int)v, bits);
  }
  hex("bits", m.data, m.cursize);

  /* 2. strings, bytes, shorts, longs, floats, angles */
  memset(buf, 0, sizeof(buf)); /* MSG_WriteBits counts a byte it has not cleared yet */
    MSG_Init(&m, buf, sizeof(buf));
  for (i = 0; i < 200; i++) {
    char s[64];
    int n = lcg() % 40;
    for (k = 0; k < n; k++) s[k] = (char)(1 + lcg() % 255);
    s[n] = 0;
    MSG_WriteString(&m, s);
    MSG_WriteByte(&m, lcg() % 256);
    MSG_WriteShort(&m, (int)(lcg() % 65536) - 32768);
    MSG_WriteLong(&m, (int)lcg2());
    MSG_WriteFloat(&m, (float)((int)(lcg() % 100000) - 50000) / 16.0f);
    MSG_WriteAngle16(&m, (float)(lcg() % 3600) / 10.0f);
  }
  hex("str", m.data, m.cursize);

  /* 3. keyed usercmd deltas */
  memset(buf, 0, sizeof(buf)); /* MSG_WriteBits counts a byte it has not cleared yet */
    MSG_Init(&m, buf, sizeof(buf));
  {
    usercmd_t a, b;
    memset(&a, 0, sizeof(a));
    for (i = 0; i < 400; i++) {
      int key = (int)lcg2();
      gen_cmd(&b, a.serverTime);
      if (lcg() % 4 == 0) b = a, b.serverTime += 16;
      MSG_WriteDeltaUsercmdKey(&m, key, &a, &b);
      a = b;
    }
  }
  hex("cmd", m.data, m.cursize);

  /* 4. entityState deltas (from baseline / from previous / removes / forced) */
  for (k = 0; k < 40; k++) {
    entityState_t a, b;
    memset(buf, 0, sizeof(buf)); /* MSG_WriteBits counts a byte it has not cleared yet */
    MSG_Init(&m, buf, sizeof(buf));
    for (i = 0; i < 20; i++) {
      gen_es(&a);
      gen_es(&b);
      b.number = a.number;
      switch (lcg() % 4) {
        case 0: MSG_WriteDeltaEntity(&m, &a, &b, qfalse); break;
        case 1: MSG_WriteDeltaEntity(&m, &a, &a, qtrue); break;
        case 2: MSG_WriteDeltaEntity(&m, &a, NULL, qtrue); break;
        default: { entityState_t z; memset(&z, 0, sizeof(z)); MSG_WriteDeltaEntity(&m, &z, &b, qtrue); }
      }
    }
    hex("ent", m.data, m.cursize);
  }

  /* 5. playerState deltas */
  for (k = 0; k < 40; k++) {
    playerState_t a, b;
    memset(buf, 0, sizeof(buf)); /* MSG_WriteBits counts a byte it has not cleared yet */
    MSG_Init(&m, buf, sizeof(buf));
    gen_ps(&a);
    gen_ps(&b);
    if (lcg() % 3 == 0) MSG_WriteDeltaPlayerstate(&m, NULL, &b);
    else MSG_WriteDeltaPlayerstate(&m, &a, &b);
    hex("ps", m.data, m.cursize);
  }

  /* 6. adaptive Huffman of a connect packet */
  {
    static byte s[MAX_MSGLEN * 2];
    const char *text = "\377\377\377\377connect \"\\cl_guid\\0123456789ABCDEF0123456789ABCDEF\\name\\Player\\rate\\25000\\snaps\\20\\protocol\\71\\qport\\4242\\challenge\\-1234567\"";
    int n = (int)strlen(text);
    memcpy(s, text, n);
    m.data = s; m.cursize = n; m.maxsize = sizeof(s);
    Huff_Compress(&m, 12);
    hex("huff", m.data, m.cursize);
    Huff_Decompress(&m, 12);
    hex("unhuff", m.data, m.cursize);
  }

  /* 7. netchan transmit with fragments (client side: qport) */
  {
    netchan_t chan;
    netadr_t adr;
    static byte data[MAX_MSGLEN];
    int lens[] = { 0, 100, 1299, 1300, 2600, 4000, 16000 };
    memset(&adr, 0, sizeof(adr));
    adr.type = NA_IP;
    Netchan_Setup(NS_CLIENT, &chan, adr, 4242, -1234567);
    for (k = 0; k < 7; k++) {
      for (i = 0; i < lens[k]; i++) data[i] = (byte)lcg();
      Netchan_Transmit(&chan, lens[k], data);
      while (chan.unsentFragments) Netchan_TransmitNextFragment(&chan);
    }
    Netchan_Setup(NS_SERVER, &chan, adr, 4242, 98765);
    for (i = 0; i < 3000; i++) data[i] = (byte)lcg();
    Netchan_Transmit(&chan, 3000, data);
    while (chan.unsentFragments) Netchan_TransmitNextFragment(&chan);
  }
  return 0;
}
