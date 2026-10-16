# Standalone cloth shop

`DF_GAMEPLAY_ADVENTURE=merchant-shop` loads this authored social slice before random
chapter generation. The pack shares only the core character creation and SRD
mechanical profile. Its scene, cast, objects and NPC dossiers are authored here.

The pinned script uses `mode: freeplay`. Its validated graph has a gathering state,
an entry into the shop, and a repeating observation action. It opens no story thread,
creates no courier, offers no combat or resolution action, and has no end card.
Native scene interactions persist independently in the existing checkpoint journal.

Marek (`merchant`), Corin (`shop-adventurer`) and Jessa (`shop-soldier`) keep the same
IDs in the scene model, public cast and their role-prefixed private dossier IDs.
Public room knowledge excludes the dossiers' trust-gated and forbidden details.

The finite `clothes-travelers` offer costs exactly 200 CP. A committed purchase
creates `purchased-clothes-travelers`; replay and item cards resolve that durable
thing. Catalog entries alone never establish ownership. Sold offers are removed
from public stock and NPC wares, while the authoritative catalog retains the key
to refuse another purchase without charging again.

`scene-mareks-cloth-shop` is the fresh room reference identity. Until its own art
is available, the asset remains pending; startup never substitutes a harbor room.
The three documentary portrait and sprite identities are the approved cast photos.

Boundary coverage is in `adventure::shop::tests`: production pack loading and record
reload, native hero creation and entry, real dossier targeting, a normal rolled
observation, take/return, purchase/reconnect/examine, current card chronology and
finite stock. Runtime, browser, media and independent evaluation remain separate gates.
