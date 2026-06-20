# Finder on MacOS

## Editing a picture with Preview

Editing the picture at path `Onewheel/le_mien_de_Onewheel/phare.jpg` fails and uploads one or more pictures in a temp subdirectory (removed sidecar
logs) :

The edited picture appears with the tag `Projets.Onewheel.le_mien_de_Onewheel.phare_jpg_sb_93035015_3rqb93` and
`Projets.Onewheel.le_mien_de_Onewheel.phare_jpg_sb_93035015_hHIYV9`
(Finder tried to add it two times). Since the temp dir name changed, finder couldn’t find the picture anymore. Tried two times then failed.
Otherwise, it would probably have emitted a move or a new PUT to replace the original picture with the temp file.

Temp directories should be detected and managed like sidecar files (though they may contain full-size pictures. This seems to be the versioning system
of Preview).

```
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-oDc68c
 WARN archypix_back::infra::error: client error status=404 error=NotFound
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93
 WARN archypix_back::infra::error: client error status=404 error=NotFound
DEBUG archypix_back::api::webdav: webdav MKCOL user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93
TRACE archypix_back::services::vfs: vfs mkcol: recorded pending mirror sub-directory user_id=dfdba256-9a6f-4136-8469-2141f1e31da0 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
 WARN archypix_back::infra::error: client error status=404 error=NotFound
DEBUG archypix_back::api::webdav: webdav PUT user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
TRACE archypix_back::services::vfs: vfs put: empty body — accepted without ingesting user_id=dfdba256-9a6f-4136-8469-2141f1e31da0 name=phare.jpg
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93
DEBUG archypix_back::api::webdav: webdav LOCK user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
DEBUG archypix_back::api::webdav: webdav UNLOCK user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
DEBUG archypix_back::api::webdav: webdav LOCK user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
DEBUG archypix_back::api::webdav: webdav PUT user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
TRACE archypix_back::services::vfs: vfs put: ingest new picture user_id=dfdba256-9a6f-4136-8469-2141f1e31da0 picture_id=8247db9b-2916-41df-b11c-1eb7ae4a5b59 name=phare.jpg bytes=2032838
TRACE archypix_back::services::vfs: vfs: apply onAdd ops user_id=dfdba256-9a6f-4136-8469-2141f1e31da0 picture_id=8247db9b-2916-41df-b11c-1eb7ae4a5b59 assigns=["Projets.Onewheel.le_mien_de_Onewheel.phare_jpg_sb_93035015_3rqb93"] removes=[]
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
 WARN archypix_back::infra::error: client error status=404 error=NotFound
DEBUG archypix_back::infra::pipeline::evaluation: pipeline: evaluating dirty pictures user_id=dfdba256-9a6f-4136-8469-2141f1e31da0 picture_count=1
DEBUG archypix_back::api::webdav: webdav UNLOCK user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
 WARN archypix_back::infra::error: client error status=404 error=NotFound
DEBUG archypix_back::infra::pipeline::evaluation: pipeline: sweep complete for user user_id=dfdba256-9a6f-4136-8469-2141f1e31da0
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93/phare.jpg
 WARN archypix_back::infra::error: client error status=404 error=NotFound
DEBUG archypix_back::api::webdav: webdav PROPFIND user=alice token_type="webdav" hierarchy=3993d23b-c4f1-47ac-acde-f938bbab17e1 path=Onewheel/le_mien_de_Onewheel/phare.jpg.sb-93035015-3rqb93
```
