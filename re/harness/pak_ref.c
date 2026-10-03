/* Pure-pak oracle (dec037): the ORIGINAL qcommon/unzip.c (minizip over
 * ioapi.c and zlib) and qcommon/md4.c (Com_BlockChecksum) compiled natively,
 * with the checksum loop of files.c FS_LoadZipFile copied verbatim. For every
 * pk3 given and a fixed set of checksum feeds it prints
 *   <basename> <numfiles> <checksum> <pure_checksum(feed)>...
 * plus Com_BlockChecksum of a few byte strings (md4 test vectors).
 * crates/trem_sim/src/kernel/files.rs computes the same; the output for
 * godot/base/data-1.1.0.pk3 is hashed into re/traces/pak.sha256 and checked by
 * crates/trem_sim/tests/pure.rs.
 * usage: pak_ref <pk3>... | sha256sum */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include "q_shared.h"
#include "qcommon.h"
#include "unzip.h"

/* ---- engine shims -------------------------------------------------- */
void QDECL Com_Printf(const char *fmt, ...) { (void)fmt; }
void QDECL Com_DPrintf(const char *fmt, ...) { (void)fmt; }
void QDECL Com_Error(int level, const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); vfprintf(stderr, fmt, ap); va_end(ap);
  (void)level; exit(2);
}
void *S_MallocDebug(int size, char *l, char *f, int ln) { (void)l; (void)f; (void)ln; return calloc(1, size); }
void *Z_MallocDebug(int size, char *l, char *f, int ln) { (void)l; (void)f; (void)ln; return calloc(1, size); }
void Z_Free(void *p) { free(p); }

#define MAX_ZPATH 256 /* files.c */
static int fs_checksumFeed;

/* files.c FS_LoadZipFile: the header longs and the two checksums */
static void load_zip(const char *zipfile, int *numfiles, int *checksum, int *pure_checksum)
{
	unzFile			uf;
	int				err;
	unz_global_info gi;
	char			filename_inzip[MAX_ZPATH];
	unz_file_info	file_info;
	int				i;
	int				fs_numHeaderLongs;
	int				*fs_headerLongs;

	fs_numHeaderLongs = 0;

	uf = unzOpen(zipfile);
	err = unzGetGlobalInfo (uf,&gi);

	if (err != UNZ_OK) {
		fprintf(stderr, "%s: not a zip\n", zipfile);
		exit(1);
	}

	fs_headerLongs = Z_Malloc( ( gi.number_entry + 1 ) * sizeof(int) );
	fs_headerLongs[ fs_numHeaderLongs++ ] = LittleLong( fs_checksumFeed );

	unzGoToFirstFile(uf);

	for (i = 0; i < gi.number_entry; i++)
	{
		err = unzGetCurrentFileInfo(uf, &file_info, filename_inzip, sizeof(filename_inzip), NULL, 0, NULL, 0);
		if (err != UNZ_OK) {
			break;
		}
		if (file_info.uncompressed_size > 0) {
			fs_headerLongs[fs_numHeaderLongs++] = LittleLong(file_info.crc);
		}
		unzGoToNextFile(uf);
	}

	*checksum = Com_BlockChecksum( &fs_headerLongs[ 1 ], sizeof(*fs_headerLongs) * ( fs_numHeaderLongs - 1 ) );
	*pure_checksum = Com_BlockChecksum( fs_headerLongs, sizeof(*fs_headerLongs) * fs_numHeaderLongs );
	*checksum = LittleLong( *checksum );
	*pure_checksum = LittleLong( *pure_checksum );
	*numfiles = gi.number_entry;

	Z_Free(fs_headerLongs);
	unzClose(uf);
}

int main(int argc, char **argv)
{
	static const int feeds[] = { 0, 1, -1, 12345, 0x7fffffff, (int)0x80000000, -559038737, 987654321 };
	static const char *vectors[] = { "", "a", "abc", "message digest", "abcdefghijklmnopqrstuvwxyz",
		"12345678901234567890123456789012345678901234567890123456789012345678901234567890" };
	int i, j;

	for (i = 0; i < (int)(sizeof(vectors) / sizeof(vectors[0])); i++)
		printf("md4 \"%s\" %u\n", vectors[i], Com_BlockChecksum(vectors[i], strlen(vectors[i])));

	for (i = 1; i < argc; i++) {
		const char *base = strrchr(argv[i], '/');
		int n, sum, pure;
		base = base ? base + 1 : argv[i];
		fs_checksumFeed = 0;
		load_zip(argv[i], &n, &sum, &pure);
		printf("%s %d %i", base, n, sum);
		for (j = 0; j < (int)(sizeof(feeds) / sizeof(feeds[0])); j++) {
			fs_checksumFeed = feeds[j];
			load_zip(argv[i], &n, &sum, &pure);
			printf(" %i", pure);
		}
		printf("\n");
	}
	return 0;
}
