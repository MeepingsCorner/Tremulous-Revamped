/* Protocol 69 network oracle (dec040): Tremulous 1.1.0's qcommon/msg.c,
 * huffman.c and net_chan.c (re/source11) compiled natively, plus
 * client/cl_net_chan.c's CL_Netchan_Encode / CL_Netchan_Decode copied
 * verbatim below (cl_net_chan.c needs client.h, whose renderer headers
 * re/source11 does not carry). Emits, as hex lines, the bytes of a
 * deterministic sequence of keyed usercmd deltas (Com_HashKey keys),
 * 1.1 entityState and playerState deltas, encoded client and decoded
 * server messages and netchan packets. crates/trem_sim/tests/net_v11.rs
 * produces the same sequence with the Rust port and compares the SHA-256
 * with re/traces/msg69.sha256.
 * usage: msg69_ref | sha256sum */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include "q_shared.h"
#include "qcommon.h"

/* ---- engine shims -------------------------------------------------- */
cvar_t *cl_shownet = NULL;
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
qboolean Sys_StringToAdr(const char *s, netadr_t *a) { (void)s; (void)a; return qfalse; }
static void hex(const char *tag, const byte *d, int n) {
  printf("%s %d ", tag, n);
  for (int i = 0; i < n; i++) printf("%02x", d[i]);
  printf("\n");
}
void Sys_SendPacket(int length, const void *data, netadr_t to) {
  (void)to;
  hex("pkt", (const byte *)data, length);
}

/* common.c of 1.1.0 */
int Com_HashKey(char *string, int maxlen) {
	int register hash, i;

	hash = 0;
	for (i = 0; i < maxlen && string[i] != '\0'; i++) {
		hash += string[i] * (119 + i);
	}
	hash = (hash ^ (hash >> 10) ^ (hash >> 20));
	return hash;
}

/* the clientConnection_t members the coders read */
static struct {
  int challenge;
  char serverCommands[MAX_RELIABLE_COMMANDS][MAX_STRING_CHARS];
  char reliableCommands[MAX_RELIABLE_COMMANDS][MAX_STRING_CHARS];
} clc;

/* ---- client/cl_net_chan.c of 1.1.0, verbatim ------------------------ */
/*
==============
CL_Netchan_Encode

	// first 12 bytes of the data are always:
	long serverId;
	long messageAcknowledge;
	long reliableAcknowledge;

==============
*/
static void CL_Netchan_Encode( msg_t *msg ) {
	int serverId, messageAcknowledge, reliableAcknowledge;
	int i, index, srdc, sbit, soob;
	byte key, *string;

	if ( msg->cursize <= CL_ENCODE_START ) {
		return;
	}

        srdc = msg->readcount;
        sbit = msg->bit;
        soob = msg->oob;

        msg->bit = 0;
        msg->readcount = 0;
        msg->oob = 0;

        serverId = MSG_ReadLong(msg);
	messageAcknowledge = MSG_ReadLong(msg);
	reliableAcknowledge = MSG_ReadLong(msg);

        msg->oob = soob;
        msg->bit = sbit;
        msg->readcount = srdc;

	string = (byte *)clc.serverCommands[ reliableAcknowledge & (MAX_RELIABLE_COMMANDS-1) ];
	index = 0;
	//
	key = clc.challenge ^ serverId ^ messageAcknowledge;
	for (i = CL_ENCODE_START; i < msg->cursize; i++) {
		// modify the key with the last received now acknowledged server command
		if (!string[index])
			index = 0;
		if (string[index] > 127 || string[index] == '%') {
			key ^= '.' << (i & 1);
		}
		else {
			key ^= string[index] << (i & 1);
		}
		index++;
		// encode the data with this key
		*(msg->data + i) = (*(msg->data + i)) ^ key;
	}
}

/*
==============
CL_Netchan_Decode

	// first four bytes of the data are always:
	long reliableAcknowledge;

==============
*/
static void CL_Netchan_Decode( msg_t *msg ) {
	long reliableAcknowledge, i, index;
	byte key, *string;
        int	srdc, sbit, soob;

        srdc = msg->readcount;
        sbit = msg->bit;
        soob = msg->oob;

        msg->oob = 0;

	reliableAcknowledge = MSG_ReadLong(msg);

        msg->oob = soob;
        msg->bit = sbit;
        msg->readcount = srdc;

	string = (byte *) clc.reliableCommands[ reliableAcknowledge & (MAX_RELIABLE_COMMANDS-1) ];
	index = 0;
	// xor the client challenge with the netchan sequence number (need something that changes every message)
	key = clc.challenge ^ LittleLong( *(unsigned *)msg->data );
	for (i = msg->readcount + CL_DECODE_START; i < msg->cursize; i++) {
		// modify the key with the last sent and with this message acknowledged client command
		if (!string[index])
			index = 0;
		if (string[index] > 127 || string[index] == '%') {
			key ^= '.' << (i & 1);
		}
		else {
			key ^= string[index] << (i & 1);
		}
		index++;
		// decode the data with this key
		*(msg->data + i) = *(msg->data + i) ^ key;
	}
}

/* ---- deterministic generator shared with tests/net_v11.rs ----------- */
static unsigned int seed = 6969u;
static unsigned int lcg(void) { seed = seed * 1103515245u + 12345u; return seed >> 8; }
/* two draws, explicitly sequenced (operand order is unspecified in C) */
static unsigned int lcg2(void) { unsigned int a = lcg(); unsigned int b = lcg(); return a ^ (b << 8); }

static int gen_word(int bits) {
  unsigned int r = lcg() % 10;
  if (r < 4) return 0;
  if (bits == 0) {
    float f;
    union { float f; int i; } fi;
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

/* a command text: printable chars, '%' and U+00C0..U+00FF as UTF-8 */
static void gen_text(char *s) {
  int n = lcg() % 40, k, l = 0;
  for (k = 0; k < n; k++) {
    switch (lcg() % 8) {
      case 0: s[l++] = '%'; break;
      case 1: { int c = 0xc0 + (int)(lcg() % 0x40); s[l++] = (char)(0xc0 | (c >> 6)); s[l++] = (char)(0x80 | (c & 0x3f)); break; }
      default: s[l++] = (char)(' ' + lcg() % 95);
    }
  }
  s[l] = 0;
}

/* msg.c's netField_t (file-local typedef there) */
typedef struct { char *name; int offset; int bits; } netField_t;
extern netField_t entityStateFields[];
extern netField_t playerStateFields[];
#define NUM_ES 51
#define NUM_PS 48

static void gen_es(entityState_t *es) {
  int i;
  memset(es, 0, sizeof(*es));
  es->number = lcg() % (MAX_GENTITIES - 1);
  for (i = 0; i < NUM_ES; i++)
    *(int *)((byte *)es + entityStateFields[i].offset) = gen_word(entityStateFields[i].bits);
}
static void gen_ps(playerState_t *ps) {
  int i;
  memset(ps, 0, sizeof(*ps));
  for (i = 0; i < NUM_PS; i++)
    *(int *)((byte *)ps + playerStateFields[i].offset) = gen_word(playerStateFields[i].bits);
  for (i = 0; i < MAX_STATS; i++) ps->stats[i] = (short)gen_word(16);
  for (i = 0; i < MAX_PERSISTANT; i++) ps->persistant[i] = (short)gen_word(16);
  for (i = 0; i < MAX_WEAPONS; i++) ps->ammo[i] = (short)gen_word(16);
  for (i = 0; i < MAX_POWERUPS; i++) ps->powerups[i] = gen_word(32);
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
  if (sizeof(entityStateFields[0]) * 0 + NUM_ES + 1 != sizeof(entityState_t) / 4) { fprintf(stderr, "entityState fields\n"); return 2; }
  Netchan_Init(4242);

  /* 1. keyed usercmd deltas, Com_HashKey in the key */
  memset(buf, 0, sizeof(buf)); /* MSG_WriteBits counts a byte it has not cleared yet */
  MSG_Init(&m, buf, sizeof(buf));
  {
    usercmd_t a, b;
    char t[128];
    memset(&a, 0, sizeof(a));
    for (i = 0; i < 300; i++) {
      int key;
      gen_text(t);
      key = (int)lcg2() ^ Com_HashKey(t, 32);
      gen_cmd(&b, a.serverTime);
      if (lcg() % 4 == 0) b = a, b.serverTime += 16;
      MSG_WriteDeltaUsercmdKey(&m, key, &a, &b);
      a = b;
    }
  }
  hex("cmd", m.data, m.cursize);

  /* 2. entityState deltas (from baseline / from previous / removes / forced) */
  for (k = 0; k < 40; k++) {
    entityState_t a, b;
    memset(buf, 0, sizeof(buf));
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

  /* 3. playerState deltas */
  for (k = 0; k < 40; k++) {
    playerState_t a, b;
    memset(buf, 0, sizeof(buf));
    MSG_Init(&m, buf, sizeof(buf));
    gen_ps(&a);
    gen_ps(&b);
    if (lcg() % 3 == 0) MSG_WriteDeltaPlayerstate(&m, NULL, &b);
    else MSG_WriteDeltaPlayerstate(&m, &a, &b);
    hex("ps", m.data, m.cursize);
  }

  /* 4. CL_Netchan_Encode / CL_Netchan_Decode */
  for (k = 0; k < 30; k++) {
    static byte d[MAX_MSGLEN];
    msg_t body;
    int n, seq;
    /* CL_ParseCommandString / CL_AddReliableCommand store with Q_strncpyz,
       whose strncpy zero-fills the rest of the slot: the coders read past
       an empty command's NUL */
    memset(clc.serverCommands, 0, sizeof(clc.serverCommands));
    for (i = 0; i < MAX_RELIABLE_COMMANDS; i++) gen_text(clc.serverCommands[i]);
    /* the port's client keeps one ring for both in this test */
    memcpy(clc.reliableCommands, clc.serverCommands, sizeof(clc.serverCommands));
    clc.challenge = (int)lcg2();
    memset(buf, 0, sizeof(buf));
    MSG_Init(&m, buf, sizeof(buf));
    MSG_WriteLong(&m, (int)lcg2());
    MSG_WriteLong(&m, (int)lcg2());
    MSG_WriteLong(&m, (int)(lcg() % 1000));
    n = lcg() % 300;
    for (i = 0; i < n; i++) MSG_WriteByte(&m, lcg() % 256);
    CL_Netchan_Encode(&m);
    hex("enc", m.data, m.cursize);
    /* a server message as Netchan_Process leaves it */
    seq = (int)(lcg() % 100000);
    memset(buf, 0, sizeof(buf));
    MSG_Init(&body, buf, sizeof(buf));
    MSG_WriteLong(&body, (int)(lcg() % 1000));
    n = lcg() % 300;
    for (i = 0; i < n; i++) MSG_WriteByte(&body, lcg() % 256);
    memset(d, 0, sizeof(d));
    memcpy(d, &seq, 4);
    memcpy(d + 4, body.data, body.cursize);
    MSG_Init(&m, d, sizeof(d));
    m.cursize = body.cursize + 4;
    MSG_BeginReadingOOB(&m);
    MSG_ReadLong(&m);
    CL_Netchan_Decode(&m);
    hex("dec", m.data, m.cursize);
  }

  /* 5. netchan transmit with fragments (client side: qport, no checksum) */
  {
    netchan_t chan;
    netadr_t adr;
    static byte data[MAX_MSGLEN];
    int lens[] = { 0, 100, 1299, 1300, 2600, 4000 };
    memset(&adr, 0, sizeof(adr));
    adr.type = NA_IP;
    Netchan_Setup(NS_CLIENT, &chan, adr, 4242);
    for (k = 0; k < 6; k++) {
      for (i = 0; i < lens[k]; i++) data[i] = (byte)lcg();
      Netchan_Transmit(&chan, lens[k], data);
      while (chan.unsentFragments) Netchan_TransmitNextFragment(&chan);
    }
  }
  return 0;
}
