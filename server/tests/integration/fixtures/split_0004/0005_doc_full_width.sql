-- FEA-178: a Doc can be shown across the whole space between the navigation panel and the outline
-- (Notion's "Full width"). The setting belongs to the page and is the same for every reader; it is not
-- part of the page's content, Versions or revision (`revision_no`, `content_hash`), so changing it
-- neither conflicts with a draft that is being saved nor creates a Version. Existing and new pages
-- start with it off, which is the layout they always had.

ALTER TABLE penfii.doc_pages
    ADD COLUMN full_width boolean DEFAULT false NOT NULL;
