//! Commands about the document: read it, write it, find things in it.

use super::super::*;

impl App {
    pub(super) fn cmd_save(&mut self) -> Result<String, String> {
        if self.in_softhand() {
            self.save_picture(None)
        } else {
            self.save()
        }
    }

    pub(super) fn cmd_save_as(&mut self, name: &str) -> Result<String, String> {
        if self.in_softhand() {
            self.save_picture(Some(name))
        } else {
            self.save_as(name)
        }
    }

    pub(super) fn cmd_open(&mut self, name: Option<String>) -> Result<String, String> {
        let Some(name) = name else {
            if self.in_softhand() {
                self.open_pictures();
            } else {
                self.open_browser();
            }
            return Ok(String::new());
        };
        if self.in_softhand() {
            let path = picture::resolve(&self.vault(), &name)?;
            if !path.is_file() {
                return Err(format!("no such picture: {name}"));
            }
            self.start_loading(Pending::Picture(path));
            return Ok(String::new());
        }
        self.prepare_vault();
        let path = self
            .find_document(&name)
            .ok_or_else(|| format!("no such document: {name}"))?;
        self.start_loading(Pending::Document(path));
        Ok(String::new())
    }

    pub(super) fn cmd_new(&mut self) -> Result<String, String> {
        if self.in_softhand() {
            self.start_loading(Pending::NewPicture);
        } else {
            self.start_loading(Pending::NewDocument);
        }
        Ok(String::new())
    }

    pub(super) fn cmd_delete(&mut self) -> Result<String, String> {
        if self.doc.path.is_none() {
            return Err("nothing to delete".into());
        }
        self.confirm = Some(format!("delete {}? y/n", self.doc.title()));
        Ok(String::new())
    }

    pub(super) fn cmd_undo(&mut self, redo: bool) -> Result<String, String> {
        let done = match self.program.as_mut() {
            Some(program) if redo => program.redo(),
            Some(program) => program.undo(),
            None if redo => self.doc.editor.redo(),
            None => self.doc.editor.undo(),
        };
        if !done {
            return Err(if redo {
                "nothing to redo".into()
            } else {
                "nothing to undo".into()
            });
        }
        self.dirty = true;
        Ok(if redo { "redone" } else { "undone" }.into())
    }

    pub(super) fn cmd_view(&mut self) -> Result<String, String> {
        if self.mode == Mode::View {
            self.to_write();
        } else {
            self.enter_view();
        }
        Ok(String::new())
    }

    pub(super) fn cmd_find(&mut self, query: Option<String>) -> Result<String, String> {
        self.start_search(query);
        Ok(String::new())
    }
}
