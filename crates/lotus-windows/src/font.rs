use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteFactory5, IDWriteFontCollection1, IDWriteInMemoryFontFileLoader,
};
use windows::core::{Error as WindowsError, IUnknown, Interface};

static FRAUNCES: &[u8] = include_bytes!("../assets/fonts/Fraunces.ttf");

pub(crate) struct BundledFontCollection {
    _loader: RegisteredFontLoader,
    collection: IDWriteFontCollection1,
}

struct RegisteredFontLoader {
    factory: IDWriteFactory,
    loader: IDWriteInMemoryFontFileLoader,
}

impl RegisteredFontLoader {
    fn create(factory: &IDWriteFactory5) -> Result<Self, WindowsError> {
        let base_factory: &IDWriteFactory = factory;
        // SAFETY: The factory creates and returns an owned loader interface.
        let loader =
            unsafe { factory.CreateInMemoryFontFileLoader() }.map_err(|error| {
                WindowsError::new(error.code(), "Lotus could not create its font loader")
            })?;
        // SAFETY: Both interfaces are live and originate from the same DirectWrite factory.
        unsafe { base_factory.RegisterFontFileLoader(&loader) }.map_err(|error| {
            WindowsError::new(error.code(), "Lotus could not register its font loader")
        })?;

        Ok(Self {
            factory: base_factory.clone(),
            loader,
        })
    }
}

impl Drop for RegisteredFontLoader {
    fn drop(&mut self) {
        // SAFETY: This guard unregisters the exact loader it registered with this factory.
        let _ = unsafe { self.factory.UnregisterFontFileLoader(&self.loader) };
    }
}

impl BundledFontCollection {
    pub(crate) fn create(factory: &IDWriteFactory5) -> Result<Self, WindowsError> {
        let data_size =
            u32::try_from(FRAUNCES.len()).expect("the bundled font is smaller than 4 GiB");
        let base_factory: &IDWriteFactory = factory;
        let owner: IUnknown = factory.cast().map_err(|error| {
            WindowsError::new(error.code(), "Lotus could not retain its font data")
        })?;

        let loader = RegisteredFontLoader::create(factory)?;
        // SAFETY: The static byte range is valid for data_size bytes and the owner remains live.
        let font_file = unsafe {
            loader.loader.CreateInMemoryFontFileReference(
                base_factory,
                FRAUNCES.as_ptr().cast(),
                data_size,
                &owner,
            )
        }
        .map_err(|error| {
            WindowsError::new(error.code(), "Lotus could not load its bundled font")
        })?;
        // SAFETY: The builder and font file are live DirectWrite objects from the same factory.
        let builder = unsafe { factory.CreateFontSetBuilder() }.map_err(|error| {
            WindowsError::new(error.code(), "Lotus could not create its font set")
        })?;
        // SAFETY: DirectWrite retains the supplied font file in the builder.
        unsafe { builder.AddFontFile(&font_file) }.map_err(|error| {
            WindowsError::new(error.code(), "Lotus could not add its bundled font")
        })?;
        // SAFETY: The populated builder produces an owned immutable font set.
        let font_set = unsafe { builder.CreateFontSet() }.map_err(|error| {
            WindowsError::new(error.code(), "Lotus could not finalize its font set")
        })?;
        // SAFETY: The font set and factory are compatible and remain live for this call.
        let collection = unsafe { factory.CreateFontCollectionFromFontSet(&font_set) }
            .map_err(|error| {
                WindowsError::new(
                    error.code(),
                    "Lotus could not create its font collection",
                )
            })?;

        Ok(Self {
            _loader: loader,
            collection,
        })
    }

    pub(crate) fn collection(&self) -> &IDWriteFontCollection1 {
        &self.collection
    }
}
