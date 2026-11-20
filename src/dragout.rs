//! Drag file dari shelf keluar ke Explorer / app lain (OLE DoDragDrop).
use std::path::{Path, PathBuf};
use windows::{
    core::{implement, BOOL, HRESULT, HSTRING},
    Win32::{
        Foundation::S_OK,
        System::{
            Com::{CoTaskMemFree, IDataObject},
            Ole::{
                DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK,
                DROPEFFECT_MOVE,
            },
            SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
        },
        UI::Shell::{
            BHID_DataObject, Common::ITEMIDLIST, IShellItemArray, SHCreateShellItemArrayFromIDLists,
            SHParseDisplayName,
        },
    },
};

const DRAGDROP_S_DROP: HRESULT = HRESULT(0x0004_0100);
const DRAGDROP_S_CANCEL: HRESULT = HRESULT(0x0004_0101);
const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = HRESULT(0x0004_0102);

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Mulai drag file (blocking sampai drop/cancel). Mengembalikan efek drop kalau berhasil.
pub fn drag_files(paths: &[PathBuf]) -> Option<DROPEFFECT> {
    unsafe {
        let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
        for p in paths {
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            let name = HSTRING::from(Path::new(p).as_os_str());
            if SHParseDisplayName(&name, None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
                pidls.push(pidl);
            }
        }
        if pidls.is_empty() {
            return None;
        }
        let res = (|| {
            let arr: IShellItemArray = SHCreateShellItemArrayFromIDLists(&pidls).ok()?;
            let data: IDataObject = arr.BindToHandler(None, &BHID_DataObject).ok()?;
            let src: IDropSource = Source.into();
            let mut eff = DROPEFFECT(0);
            let hr = DoDragDrop(&data, &src, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut eff);
            (hr == DRAGDROP_S_DROP).then_some(eff)
        })();
        for p in pidls {
            CoTaskMemFree(Some(p as *const _));
        }
        res
    }
}
