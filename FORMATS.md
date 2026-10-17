# Sfoglia format support

Generated from `.zen/contract.md` by `cargo run -p xtask -- formats`. Do not edit by hand: `cargo test` fails while this file is stale.

Legend: **done** = proven by a verified clause. **in progress** = clause exists, not yet verified. **planned** = pending clause. **later** = a later extension of the clause. **no** = out of scope.

17 families, 389 extensions.

| Family | Clause | Read | Write | Requested | Extensions |
|---|---|---|---|---|---|
| Source and text viewing with syntax highlighting | C-005 | done | no | user request 2026-10-13 | txt, log, text |
| Images | C-009 | done | done | user request 2026-10-13 | aai, apng, avif, bmp, cin, dcm, dicom, dds, dfont, dpx, emf, emz, exr, ff, gif, hdr, heic, heif, ico, jls, jng, jp2, jpc, jpx, jpg, jpeg, jfif, jxl, jxr, ora, pbm, pcd, pes, png, psb, psd, qoi, sfw, svg, svgz, tga, targa, tiff, tif, webp, wmf, wmz, bw, cdg, cur, dib, miff, mvg, pcx, pgm, pix, pam, pnm, ppm, ptx, rgb, rgba, sgi, ttf, txd, wbmp, xbm, xface, xpm |
| Structured data trees | C-010 | done | done | user request 2026-10-13 | json, jsonl, xml, yaml, yml, toml, ini |
| Markdown rendering | C-011 | done | done | user request 2026-10-13 | md, markdown |
| Word-processing documents | P-007 | planned | later | user request 2026-10-13 | doc, docm, docx, dot, dotm, dotx, epub, odt, ott, rtf, wps |
| PDF and XPS viewing | P-008 | planned | no | user request 2026-10-13 | pdf, oxps, xps |
| Spreadsheets and delimited tables | P-010 | planned | planned | user request 2026-10-13 | csv, tsv, xls, xlt, xlsm, xlsx, xltx, xltm |
| Presentations | P-011 | planned | no | user request 2026-10-13 | odp, pot, potm, potx, pps, ppsx, ppt, pptm, pptx |
| Visio diagrams | P-012 | planned | no | user request 2026-10-13 | vdw, vdx, vsd, vsdm, vsdx, vss, vssx, vst, vstm, vstx, vsx, vtx |
| Microsoft Project files | P-013 | planned | no | user request 2026-10-13 | mpp, mpt, mpx |
| Email messages | P-014 | planned | no | user request 2026-10-13 | eml, emlx, msg, oft, dat |
| Web pages | P-015 | planned | planned | user request 2026-10-13 | html, htm, xhtml, mht, mhtml |
| CAD drawings | P-016 | planned | planned | user request 2026-10-13 | dwg, dxf |
| Camera RAW images | P-018 | planned | no | user request 2026-10-13 | 3fr, ari, arw, bay, cr2, cr3, crw, dcr, dng, erf, fff, iiq, kc2, kdc, mdc, mef, mos, mrw, nef, nrw, orf, pef, raf, raw, rw2, rwl, sr2, srf, srw, x3f |
| Audio playback | P-019 | planned | no | user request 2026-10-13 | 3ga, 3gpa, aa, aa3, at3, aac, ac3, aif, aiff, aifc, amr, ape, caf, caff, dss, dts, flac, gsm, m4a, m4b, m4r, mka, mlp, mp1, mp2, mp3, ogg, oga, oma, opus, ra, w64, wav, wma, wv, xa, xma, 8svx, act, aea, apc, au, aud, g722, htk, iss, rso, sf, ircam, mpa, mpc, paf, fap, pvf, qcp, shn, snd, sol, son, sph, nist, spx, str, tak, tta, voc, vqf |
| Video playback through VLC | P-020 | planned | no | user request 2026-10-13 | 3g2, 3gp, asf, avi, bik, cam, ml20, dat, divx, dvr-ms, f4v, flv, gxf, hevc, m2t, m2ts, m2v, m4v, mk3d, mkv, mod, mov, mp4, mpg, mpeg, mts, mxf, nsv, nut, nuv, ogm, ogv, pmf, pmp, rm, rmvb, thp, tp, trp, ts, vob, webm, wmv, wtv, xesc, xmv, 4xm, amv, anim, anm, bfi, bmv, c93, cak, cmv, cpk, cin, cdxl, dct, dfa, drc, duk, dv, dxa, fli, flc, flic, gdv, h263, h264, jv, k3g, skm, lvf, lxf, mad, mmv, mv, mv8, mve, mpc, mvi, p64, pva, qt, rl2, roq, rpl, san, snm, sfd, smk, tgq, tgv, tmv, vb, vc1, vid, vmd, vp3, vp5, vp6, vp7, vqa, vsr, wve, yop |
| Archives, with 7z read and write, as the seed of an archive manager | P-021 | planned | planned | user request 2026-10-13 | 7z, apk, arj, bz2, bzip2, cab, cbr, cbz, chm, cpio, dd, deb, deskthemepack, dmg, gz, gzip, hfs, iso, jar, lha, lzh, lzma, pk3, pk4, pkz, rar, rpm, tar, tar.lzma, taz, tar.z, tbz, tar.bz2, tbz2, tgz, tar.gz, txz, tar.xz, vhd, war, wim, xar, xz, z, zip, zipx |
