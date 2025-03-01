// @generated
/// Marks a type as a data provider. You can then use macros like
/// `impl_core_helloworld_v1` to add implementations.
///
/// ```ignore
/// struct MyProvider;
/// const _: () = {
///     include!("path/to/generated/macros.rs");
///     make_provider!(MyProvider);
///     impl_core_helloworld_v1!(MyProvider);
/// }
/// ```
#[doc(hidden)]
#[macro_export]
macro_rules! __make_provider {
    ($ name : ty) => {
        #[clippy::msrv = "1.67"]
        impl $name {
            #[doc(hidden)]
            #[allow(dead_code)]
            pub const MUST_USE_MAKE_PROVIDER_MACRO: () = ();
        }
        icu_provider::impl_data_provider_never_marker!($name);
    };
}
#[doc(inline)]
pub use __make_provider as make_provider;
#[macro_use]
#[path = "macros/propnames_from_gcb_v1.rs.data"]
mod propnames_from_gcb_v1;
#[doc(inline)]
pub use __impl_propnames_from_gcb_v1 as impl_propnames_from_gcb_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_gcb_v1 as impliterable_propnames_from_gcb_v1;
#[macro_use]
#[path = "macros/propnames_from_insc_v1.rs.data"]
mod propnames_from_insc_v1;
#[doc(inline)]
pub use __impl_propnames_from_insc_v1 as impl_propnames_from_insc_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_insc_v1 as impliterable_propnames_from_insc_v1;
#[macro_use]
#[path = "macros/propnames_from_sb_v1.rs.data"]
mod propnames_from_sb_v1;
#[doc(inline)]
pub use __impl_propnames_from_sb_v1 as impl_propnames_from_sb_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_sb_v1 as impliterable_propnames_from_sb_v1;
#[macro_use]
#[path = "macros/propnames_from_wb_v1.rs.data"]
mod propnames_from_wb_v1;
#[doc(inline)]
pub use __impl_propnames_from_wb_v1 as impl_propnames_from_wb_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_wb_v1 as impliterable_propnames_from_wb_v1;
#[macro_use]
#[path = "macros/propnames_from_bc_v1.rs.data"]
mod propnames_from_bc_v1;
#[doc(inline)]
pub use __impl_propnames_from_bc_v1 as impl_propnames_from_bc_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_bc_v1 as impliterable_propnames_from_bc_v1;
#[macro_use]
#[path = "macros/propnames_from_ccc_v1.rs.data"]
mod propnames_from_ccc_v1;
#[doc(inline)]
pub use __impl_propnames_from_ccc_v1 as impl_propnames_from_ccc_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_ccc_v1 as impliterable_propnames_from_ccc_v1;
#[macro_use]
#[path = "macros/propnames_from_ea_v1.rs.data"]
mod propnames_from_ea_v1;
#[doc(inline)]
pub use __impl_propnames_from_ea_v1 as impl_propnames_from_ea_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_ea_v1 as impliterable_propnames_from_ea_v1;
#[macro_use]
#[path = "macros/propnames_from_gc_v1.rs.data"]
mod propnames_from_gc_v1;
#[doc(inline)]
pub use __impl_propnames_from_gc_v1 as impl_propnames_from_gc_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_gc_v1 as impliterable_propnames_from_gc_v1;
#[macro_use]
#[path = "macros/propnames_from_gcm_v1.rs.data"]
mod propnames_from_gcm_v1;
#[doc(inline)]
pub use __impl_propnames_from_gcm_v1 as impl_propnames_from_gcm_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_gcm_v1 as impliterable_propnames_from_gcm_v1;
#[macro_use]
#[path = "macros/propnames_from_hst_v1.rs.data"]
mod propnames_from_hst_v1;
#[doc(inline)]
pub use __impl_propnames_from_hst_v1 as impl_propnames_from_hst_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_hst_v1 as impliterable_propnames_from_hst_v1;
#[macro_use]
#[path = "macros/propnames_from_jt_v1.rs.data"]
mod propnames_from_jt_v1;
#[doc(inline)]
pub use __impl_propnames_from_jt_v1 as impl_propnames_from_jt_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_jt_v1 as impliterable_propnames_from_jt_v1;
#[macro_use]
#[path = "macros/propnames_from_lb_v1.rs.data"]
mod propnames_from_lb_v1;
#[doc(inline)]
pub use __impl_propnames_from_lb_v1 as impl_propnames_from_lb_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_lb_v1 as impliterable_propnames_from_lb_v1;
#[macro_use]
#[path = "macros/propnames_from_sc_v1.rs.data"]
mod propnames_from_sc_v1;
#[doc(inline)]
pub use __impl_propnames_from_sc_v1 as impl_propnames_from_sc_v1;
#[doc(inline)]
pub use __impliterable_propnames_from_sc_v1 as impliterable_propnames_from_sc_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_gcb_v1.rs.data"]
mod propnames_to_long_linear_gcb_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_gcb_v1 as impl_propnames_to_long_linear_gcb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_gcb_v1 as impliterable_propnames_to_long_linear_gcb_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_insc_v1.rs.data"]
mod propnames_to_long_linear_insc_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_insc_v1 as impl_propnames_to_long_linear_insc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_insc_v1 as impliterable_propnames_to_long_linear_insc_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_sb_v1.rs.data"]
mod propnames_to_long_linear_sb_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_sb_v1 as impl_propnames_to_long_linear_sb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_sb_v1 as impliterable_propnames_to_long_linear_sb_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_wb_v1.rs.data"]
mod propnames_to_long_linear_wb_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_wb_v1 as impl_propnames_to_long_linear_wb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_wb_v1 as impliterable_propnames_to_long_linear_wb_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_bc_v1.rs.data"]
mod propnames_to_long_linear_bc_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_bc_v1 as impl_propnames_to_long_linear_bc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_bc_v1 as impliterable_propnames_to_long_linear_bc_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_ea_v1.rs.data"]
mod propnames_to_long_linear_ea_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_ea_v1 as impl_propnames_to_long_linear_ea_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_ea_v1 as impliterable_propnames_to_long_linear_ea_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_gc_v1.rs.data"]
mod propnames_to_long_linear_gc_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_gc_v1 as impl_propnames_to_long_linear_gc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_gc_v1 as impliterable_propnames_to_long_linear_gc_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_hst_v1.rs.data"]
mod propnames_to_long_linear_hst_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_hst_v1 as impl_propnames_to_long_linear_hst_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_hst_v1 as impliterable_propnames_to_long_linear_hst_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_jt_v1.rs.data"]
mod propnames_to_long_linear_jt_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_jt_v1 as impl_propnames_to_long_linear_jt_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_jt_v1 as impliterable_propnames_to_long_linear_jt_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_lb_v1.rs.data"]
mod propnames_to_long_linear_lb_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_lb_v1 as impl_propnames_to_long_linear_lb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_lb_v1 as impliterable_propnames_to_long_linear_lb_v1;
#[macro_use]
#[path = "macros/propnames_to_long_linear_sc_v1.rs.data"]
mod propnames_to_long_linear_sc_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_linear_sc_v1 as impl_propnames_to_long_linear_sc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_linear_sc_v1 as impliterable_propnames_to_long_linear_sc_v1;
#[macro_use]
#[path = "macros/propnames_to_long_sparse_ccc_v1.rs.data"]
mod propnames_to_long_sparse_ccc_v1;
#[doc(inline)]
pub use __impl_propnames_to_long_sparse_ccc_v1 as impl_propnames_to_long_sparse_ccc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_long_sparse_ccc_v1 as impliterable_propnames_to_long_sparse_ccc_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_gcb_v1.rs.data"]
mod propnames_to_short_linear_gcb_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_gcb_v1 as impl_propnames_to_short_linear_gcb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_gcb_v1 as impliterable_propnames_to_short_linear_gcb_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_insc_v1.rs.data"]
mod propnames_to_short_linear_insc_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_insc_v1 as impl_propnames_to_short_linear_insc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_insc_v1 as impliterable_propnames_to_short_linear_insc_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_sb_v1.rs.data"]
mod propnames_to_short_linear_sb_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_sb_v1 as impl_propnames_to_short_linear_sb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_sb_v1 as impliterable_propnames_to_short_linear_sb_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_wb_v1.rs.data"]
mod propnames_to_short_linear_wb_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_wb_v1 as impl_propnames_to_short_linear_wb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_wb_v1 as impliterable_propnames_to_short_linear_wb_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_bc_v1.rs.data"]
mod propnames_to_short_linear_bc_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_bc_v1 as impl_propnames_to_short_linear_bc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_bc_v1 as impliterable_propnames_to_short_linear_bc_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_ea_v1.rs.data"]
mod propnames_to_short_linear_ea_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_ea_v1 as impl_propnames_to_short_linear_ea_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_ea_v1 as impliterable_propnames_to_short_linear_ea_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_gc_v1.rs.data"]
mod propnames_to_short_linear_gc_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_gc_v1 as impl_propnames_to_short_linear_gc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_gc_v1 as impliterable_propnames_to_short_linear_gc_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_hst_v1.rs.data"]
mod propnames_to_short_linear_hst_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_hst_v1 as impl_propnames_to_short_linear_hst_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_hst_v1 as impliterable_propnames_to_short_linear_hst_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_jt_v1.rs.data"]
mod propnames_to_short_linear_jt_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_jt_v1 as impl_propnames_to_short_linear_jt_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_jt_v1 as impliterable_propnames_to_short_linear_jt_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear_lb_v1.rs.data"]
mod propnames_to_short_linear_lb_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear_lb_v1 as impl_propnames_to_short_linear_lb_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear_lb_v1 as impliterable_propnames_to_short_linear_lb_v1;
#[macro_use]
#[path = "macros/propnames_to_short_linear4_sc_v1.rs.data"]
mod propnames_to_short_linear4_sc_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_linear4_sc_v1 as impl_propnames_to_short_linear4_sc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_linear4_sc_v1 as impliterable_propnames_to_short_linear4_sc_v1;
#[macro_use]
#[path = "macros/propnames_to_short_sparse_ccc_v1.rs.data"]
mod propnames_to_short_sparse_ccc_v1;
#[doc(inline)]
pub use __impl_propnames_to_short_sparse_ccc_v1 as impl_propnames_to_short_sparse_ccc_v1;
#[doc(inline)]
pub use __impliterable_propnames_to_short_sparse_ccc_v1 as impliterable_propnames_to_short_sparse_ccc_v1;
#[macro_use]
#[path = "macros/props_ahex_v1.rs.data"]
mod props_ahex_v1;
#[doc(inline)]
pub use __impl_props_ahex_v1 as impl_props_ahex_v1;
#[doc(inline)]
pub use __impliterable_props_ahex_v1 as impliterable_props_ahex_v1;
#[macro_use]
#[path = "macros/props_alpha_v1.rs.data"]
mod props_alpha_v1;
#[doc(inline)]
pub use __impl_props_alpha_v1 as impl_props_alpha_v1;
#[doc(inline)]
pub use __impliterable_props_alpha_v1 as impliterable_props_alpha_v1;
#[macro_use]
#[path = "macros/props_basic_emoji_v1.rs.data"]
mod props_basic_emoji_v1;
#[doc(inline)]
pub use __impl_props_basic_emoji_v1 as impl_props_basic_emoji_v1;
#[doc(inline)]
pub use __impliterable_props_basic_emoji_v1 as impliterable_props_basic_emoji_v1;
#[macro_use]
#[path = "macros/props_bidi_c_v1.rs.data"]
mod props_bidi_c_v1;
#[doc(inline)]
pub use __impl_props_bidi_c_v1 as impl_props_bidi_c_v1;
#[doc(inline)]
pub use __impliterable_props_bidi_c_v1 as impliterable_props_bidi_c_v1;
#[macro_use]
#[path = "macros/props_bidi_m_v1.rs.data"]
mod props_bidi_m_v1;
#[doc(inline)]
pub use __impl_props_bidi_m_v1 as impl_props_bidi_m_v1;
#[doc(inline)]
pub use __impliterable_props_bidi_m_v1 as impliterable_props_bidi_m_v1;
#[macro_use]
#[path = "macros/props_ci_v1.rs.data"]
mod props_ci_v1;
#[doc(inline)]
pub use __impl_props_ci_v1 as impl_props_ci_v1;
#[doc(inline)]
pub use __impliterable_props_ci_v1 as impliterable_props_ci_v1;
#[macro_use]
#[path = "macros/props_cwcf_v1.rs.data"]
mod props_cwcf_v1;
#[doc(inline)]
pub use __impl_props_cwcf_v1 as impl_props_cwcf_v1;
#[doc(inline)]
pub use __impliterable_props_cwcf_v1 as impliterable_props_cwcf_v1;
#[macro_use]
#[path = "macros/props_cwcm_v1.rs.data"]
mod props_cwcm_v1;
#[doc(inline)]
pub use __impl_props_cwcm_v1 as impl_props_cwcm_v1;
#[doc(inline)]
pub use __impliterable_props_cwcm_v1 as impliterable_props_cwcm_v1;
#[macro_use]
#[path = "macros/props_cwkcf_v1.rs.data"]
mod props_cwkcf_v1;
#[doc(inline)]
pub use __impl_props_cwkcf_v1 as impl_props_cwkcf_v1;
#[doc(inline)]
pub use __impliterable_props_cwkcf_v1 as impliterable_props_cwkcf_v1;
#[macro_use]
#[path = "macros/props_cwl_v1.rs.data"]
mod props_cwl_v1;
#[doc(inline)]
pub use __impl_props_cwl_v1 as impl_props_cwl_v1;
#[doc(inline)]
pub use __impliterable_props_cwl_v1 as impliterable_props_cwl_v1;
#[macro_use]
#[path = "macros/props_cwt_v1.rs.data"]
mod props_cwt_v1;
#[doc(inline)]
pub use __impl_props_cwt_v1 as impl_props_cwt_v1;
#[doc(inline)]
pub use __impliterable_props_cwt_v1 as impliterable_props_cwt_v1;
#[macro_use]
#[path = "macros/props_cwu_v1.rs.data"]
mod props_cwu_v1;
#[doc(inline)]
pub use __impl_props_cwu_v1 as impl_props_cwu_v1;
#[doc(inline)]
pub use __impliterable_props_cwu_v1 as impliterable_props_cwu_v1;
#[macro_use]
#[path = "macros/props_cased_v1.rs.data"]
mod props_cased_v1;
#[doc(inline)]
pub use __impl_props_cased_v1 as impl_props_cased_v1;
#[doc(inline)]
pub use __impliterable_props_cased_v1 as impliterable_props_cased_v1;
#[macro_use]
#[path = "macros/props_comp_ex_v1.rs.data"]
mod props_comp_ex_v1;
#[doc(inline)]
pub use __impl_props_comp_ex_v1 as impl_props_comp_ex_v1;
#[doc(inline)]
pub use __impliterable_props_comp_ex_v1 as impliterable_props_comp_ex_v1;
#[macro_use]
#[path = "macros/props_di_v1.rs.data"]
mod props_di_v1;
#[doc(inline)]
pub use __impl_props_di_v1 as impl_props_di_v1;
#[doc(inline)]
pub use __impliterable_props_di_v1 as impliterable_props_di_v1;
#[macro_use]
#[path = "macros/props_dash_v1.rs.data"]
mod props_dash_v1;
#[doc(inline)]
pub use __impl_props_dash_v1 as impl_props_dash_v1;
#[doc(inline)]
pub use __impliterable_props_dash_v1 as impliterable_props_dash_v1;
#[macro_use]
#[path = "macros/props_dep_v1.rs.data"]
mod props_dep_v1;
#[doc(inline)]
pub use __impl_props_dep_v1 as impl_props_dep_v1;
#[doc(inline)]
pub use __impliterable_props_dep_v1 as impliterable_props_dep_v1;
#[macro_use]
#[path = "macros/props_dia_v1.rs.data"]
mod props_dia_v1;
#[doc(inline)]
pub use __impl_props_dia_v1 as impl_props_dia_v1;
#[doc(inline)]
pub use __impliterable_props_dia_v1 as impliterable_props_dia_v1;
#[macro_use]
#[path = "macros/props_ebase_v1.rs.data"]
mod props_ebase_v1;
#[doc(inline)]
pub use __impl_props_ebase_v1 as impl_props_ebase_v1;
#[doc(inline)]
pub use __impliterable_props_ebase_v1 as impliterable_props_ebase_v1;
#[macro_use]
#[path = "macros/props_ecomp_v1.rs.data"]
mod props_ecomp_v1;
#[doc(inline)]
pub use __impl_props_ecomp_v1 as impl_props_ecomp_v1;
#[doc(inline)]
pub use __impliterable_props_ecomp_v1 as impliterable_props_ecomp_v1;
#[macro_use]
#[path = "macros/props_emod_v1.rs.data"]
mod props_emod_v1;
#[doc(inline)]
pub use __impl_props_emod_v1 as impl_props_emod_v1;
#[doc(inline)]
pub use __impliterable_props_emod_v1 as impliterable_props_emod_v1;
#[macro_use]
#[path = "macros/props_epres_v1.rs.data"]
mod props_epres_v1;
#[doc(inline)]
pub use __impl_props_epres_v1 as impl_props_epres_v1;
#[doc(inline)]
pub use __impliterable_props_epres_v1 as impliterable_props_epres_v1;
#[macro_use]
#[path = "macros/props_emoji_v1.rs.data"]
mod props_emoji_v1;
#[doc(inline)]
pub use __impl_props_emoji_v1 as impl_props_emoji_v1;
#[doc(inline)]
pub use __impliterable_props_emoji_v1 as impliterable_props_emoji_v1;
#[macro_use]
#[path = "macros/props_ext_v1.rs.data"]
mod props_ext_v1;
#[doc(inline)]
pub use __impl_props_ext_v1 as impl_props_ext_v1;
#[doc(inline)]
pub use __impliterable_props_ext_v1 as impliterable_props_ext_v1;
#[macro_use]
#[path = "macros/props_extpict_v1.rs.data"]
mod props_extpict_v1;
#[doc(inline)]
pub use __impl_props_extpict_v1 as impl_props_extpict_v1;
#[doc(inline)]
pub use __impliterable_props_extpict_v1 as impliterable_props_extpict_v1;
#[macro_use]
#[path = "macros/props_gcb_v1.rs.data"]
mod props_gcb_v1;
#[doc(inline)]
pub use __impl_props_gcb_v1 as impl_props_gcb_v1;
#[doc(inline)]
pub use __impliterable_props_gcb_v1 as impliterable_props_gcb_v1;
#[macro_use]
#[path = "macros/props_gr_base_v1.rs.data"]
mod props_gr_base_v1;
#[doc(inline)]
pub use __impl_props_gr_base_v1 as impl_props_gr_base_v1;
#[doc(inline)]
pub use __impliterable_props_gr_base_v1 as impliterable_props_gr_base_v1;
#[macro_use]
#[path = "macros/props_gr_ext_v1.rs.data"]
mod props_gr_ext_v1;
#[doc(inline)]
pub use __impl_props_gr_ext_v1 as impl_props_gr_ext_v1;
#[doc(inline)]
pub use __impliterable_props_gr_ext_v1 as impliterable_props_gr_ext_v1;
#[macro_use]
#[path = "macros/props_gr_link_v1.rs.data"]
mod props_gr_link_v1;
#[doc(inline)]
pub use __impl_props_gr_link_v1 as impl_props_gr_link_v1;
#[doc(inline)]
pub use __impliterable_props_gr_link_v1 as impliterable_props_gr_link_v1;
#[macro_use]
#[path = "macros/props_hex_v1.rs.data"]
mod props_hex_v1;
#[doc(inline)]
pub use __impl_props_hex_v1 as impl_props_hex_v1;
#[doc(inline)]
pub use __impliterable_props_hex_v1 as impliterable_props_hex_v1;
#[macro_use]
#[path = "macros/props_hyphen_v1.rs.data"]
mod props_hyphen_v1;
#[doc(inline)]
pub use __impl_props_hyphen_v1 as impl_props_hyphen_v1;
#[doc(inline)]
pub use __impliterable_props_hyphen_v1 as impliterable_props_hyphen_v1;
#[macro_use]
#[path = "macros/props_idc_v1.rs.data"]
mod props_idc_v1;
#[doc(inline)]
pub use __impl_props_idc_v1 as impl_props_idc_v1;
#[doc(inline)]
pub use __impliterable_props_idc_v1 as impliterable_props_idc_v1;
#[macro_use]
#[path = "macros/props_ids_v1.rs.data"]
mod props_ids_v1;
#[doc(inline)]
pub use __impl_props_ids_v1 as impl_props_ids_v1;
#[doc(inline)]
pub use __impliterable_props_ids_v1 as impliterable_props_ids_v1;
#[macro_use]
#[path = "macros/props_idsb_v1.rs.data"]
mod props_idsb_v1;
#[doc(inline)]
pub use __impl_props_idsb_v1 as impl_props_idsb_v1;
#[doc(inline)]
pub use __impliterable_props_idsb_v1 as impliterable_props_idsb_v1;
#[macro_use]
#[path = "macros/props_idst_v1.rs.data"]
mod props_idst_v1;
#[doc(inline)]
pub use __impl_props_idst_v1 as impl_props_idst_v1;
#[doc(inline)]
pub use __impliterable_props_idst_v1 as impliterable_props_idst_v1;
#[macro_use]
#[path = "macros/props_ideo_v1.rs.data"]
mod props_ideo_v1;
#[doc(inline)]
pub use __impl_props_ideo_v1 as impl_props_ideo_v1;
#[doc(inline)]
pub use __impliterable_props_ideo_v1 as impliterable_props_ideo_v1;
#[macro_use]
#[path = "macros/props_insc_v1.rs.data"]
mod props_insc_v1;
#[doc(inline)]
pub use __impl_props_insc_v1 as impl_props_insc_v1;
#[doc(inline)]
pub use __impliterable_props_insc_v1 as impliterable_props_insc_v1;
#[macro_use]
#[path = "macros/props_join_c_v1.rs.data"]
mod props_join_c_v1;
#[doc(inline)]
pub use __impl_props_join_c_v1 as impl_props_join_c_v1;
#[doc(inline)]
pub use __impliterable_props_join_c_v1 as impliterable_props_join_c_v1;
#[macro_use]
#[path = "macros/props_loe_v1.rs.data"]
mod props_loe_v1;
#[doc(inline)]
pub use __impl_props_loe_v1 as impl_props_loe_v1;
#[doc(inline)]
pub use __impliterable_props_loe_v1 as impliterable_props_loe_v1;
#[macro_use]
#[path = "macros/props_lower_v1.rs.data"]
mod props_lower_v1;
#[doc(inline)]
pub use __impl_props_lower_v1 as impl_props_lower_v1;
#[doc(inline)]
pub use __impliterable_props_lower_v1 as impliterable_props_lower_v1;
#[macro_use]
#[path = "macros/props_math_v1.rs.data"]
mod props_math_v1;
#[doc(inline)]
pub use __impl_props_math_v1 as impl_props_math_v1;
#[doc(inline)]
pub use __impliterable_props_math_v1 as impliterable_props_math_v1;
#[macro_use]
#[path = "macros/props_nchar_v1.rs.data"]
mod props_nchar_v1;
#[doc(inline)]
pub use __impl_props_nchar_v1 as impl_props_nchar_v1;
#[doc(inline)]
pub use __impliterable_props_nchar_v1 as impliterable_props_nchar_v1;
#[macro_use]
#[path = "macros/props_pcm_v1.rs.data"]
mod props_pcm_v1;
#[doc(inline)]
pub use __impl_props_pcm_v1 as impl_props_pcm_v1;
#[doc(inline)]
pub use __impliterable_props_pcm_v1 as impliterable_props_pcm_v1;
#[macro_use]
#[path = "macros/props_pat_syn_v1.rs.data"]
mod props_pat_syn_v1;
#[doc(inline)]
pub use __impl_props_pat_syn_v1 as impl_props_pat_syn_v1;
#[doc(inline)]
pub use __impliterable_props_pat_syn_v1 as impliterable_props_pat_syn_v1;
#[macro_use]
#[path = "macros/props_pat_ws_v1.rs.data"]
mod props_pat_ws_v1;
#[doc(inline)]
pub use __impl_props_pat_ws_v1 as impl_props_pat_ws_v1;
#[doc(inline)]
pub use __impliterable_props_pat_ws_v1 as impliterable_props_pat_ws_v1;
#[macro_use]
#[path = "macros/props_qmark_v1.rs.data"]
mod props_qmark_v1;
#[doc(inline)]
pub use __impl_props_qmark_v1 as impl_props_qmark_v1;
#[doc(inline)]
pub use __impliterable_props_qmark_v1 as impliterable_props_qmark_v1;
#[macro_use]
#[path = "macros/props_ri_v1.rs.data"]
mod props_ri_v1;
#[doc(inline)]
pub use __impl_props_ri_v1 as impl_props_ri_v1;
#[doc(inline)]
pub use __impliterable_props_ri_v1 as impliterable_props_ri_v1;
#[macro_use]
#[path = "macros/props_radical_v1.rs.data"]
mod props_radical_v1;
#[doc(inline)]
pub use __impl_props_radical_v1 as impl_props_radical_v1;
#[doc(inline)]
pub use __impliterable_props_radical_v1 as impliterable_props_radical_v1;
#[macro_use]
#[path = "macros/props_sb_v1.rs.data"]
mod props_sb_v1;
#[doc(inline)]
pub use __impl_props_sb_v1 as impl_props_sb_v1;
#[doc(inline)]
pub use __impliterable_props_sb_v1 as impliterable_props_sb_v1;
#[macro_use]
#[path = "macros/props_sd_v1.rs.data"]
mod props_sd_v1;
#[doc(inline)]
pub use __impl_props_sd_v1 as impl_props_sd_v1;
#[doc(inline)]
pub use __impliterable_props_sd_v1 as impliterable_props_sd_v1;
#[macro_use]
#[path = "macros/props_sterm_v1.rs.data"]
mod props_sterm_v1;
#[doc(inline)]
pub use __impl_props_sterm_v1 as impl_props_sterm_v1;
#[doc(inline)]
pub use __impliterable_props_sterm_v1 as impliterable_props_sterm_v1;
#[macro_use]
#[path = "macros/props_sensitive_v1.rs.data"]
mod props_sensitive_v1;
#[doc(inline)]
pub use __impl_props_sensitive_v1 as impl_props_sensitive_v1;
#[doc(inline)]
pub use __impliterable_props_sensitive_v1 as impliterable_props_sensitive_v1;
#[macro_use]
#[path = "macros/props_term_v1.rs.data"]
mod props_term_v1;
#[doc(inline)]
pub use __impl_props_term_v1 as impl_props_term_v1;
#[doc(inline)]
pub use __impliterable_props_term_v1 as impliterable_props_term_v1;
#[macro_use]
#[path = "macros/props_uideo_v1.rs.data"]
mod props_uideo_v1;
#[doc(inline)]
pub use __impl_props_uideo_v1 as impl_props_uideo_v1;
#[doc(inline)]
pub use __impliterable_props_uideo_v1 as impliterable_props_uideo_v1;
#[macro_use]
#[path = "macros/props_upper_v1.rs.data"]
mod props_upper_v1;
#[doc(inline)]
pub use __impl_props_upper_v1 as impl_props_upper_v1;
#[doc(inline)]
pub use __impliterable_props_upper_v1 as impliterable_props_upper_v1;
#[macro_use]
#[path = "macros/props_vs_v1.rs.data"]
mod props_vs_v1;
#[doc(inline)]
pub use __impl_props_vs_v1 as impl_props_vs_v1;
#[doc(inline)]
pub use __impliterable_props_vs_v1 as impliterable_props_vs_v1;
#[macro_use]
#[path = "macros/props_wb_v1.rs.data"]
mod props_wb_v1;
#[doc(inline)]
pub use __impl_props_wb_v1 as impl_props_wb_v1;
#[doc(inline)]
pub use __impliterable_props_wb_v1 as impliterable_props_wb_v1;
#[macro_use]
#[path = "macros/props_wspace_v1.rs.data"]
mod props_wspace_v1;
#[doc(inline)]
pub use __impl_props_wspace_v1 as impl_props_wspace_v1;
#[doc(inline)]
pub use __impliterable_props_wspace_v1 as impliterable_props_wspace_v1;
#[macro_use]
#[path = "macros/props_xidc_v1.rs.data"]
mod props_xidc_v1;
#[doc(inline)]
pub use __impl_props_xidc_v1 as impl_props_xidc_v1;
#[doc(inline)]
pub use __impliterable_props_xidc_v1 as impliterable_props_xidc_v1;
#[macro_use]
#[path = "macros/props_xids_v1.rs.data"]
mod props_xids_v1;
#[doc(inline)]
pub use __impl_props_xids_v1 as impl_props_xids_v1;
#[doc(inline)]
pub use __impliterable_props_xids_v1 as impliterable_props_xids_v1;
#[macro_use]
#[path = "macros/props_alnum_v1.rs.data"]
mod props_alnum_v1;
#[doc(inline)]
pub use __impl_props_alnum_v1 as impl_props_alnum_v1;
#[doc(inline)]
pub use __impliterable_props_alnum_v1 as impliterable_props_alnum_v1;
#[macro_use]
#[path = "macros/props_bc_v1.rs.data"]
mod props_bc_v1;
#[doc(inline)]
pub use __impl_props_bc_v1 as impl_props_bc_v1;
#[doc(inline)]
pub use __impliterable_props_bc_v1 as impliterable_props_bc_v1;
#[macro_use]
#[path = "macros/props_bidiauxiliaryprops_v1.rs.data"]
mod props_bidiauxiliaryprops_v1;
#[doc(inline)]
pub use __impl_props_bidiauxiliaryprops_v1 as impl_props_bidiauxiliaryprops_v1;
#[doc(inline)]
pub use __impliterable_props_bidiauxiliaryprops_v1 as impliterable_props_bidiauxiliaryprops_v1;
#[macro_use]
#[path = "macros/props_blank_v1.rs.data"]
mod props_blank_v1;
#[doc(inline)]
pub use __impl_props_blank_v1 as impl_props_blank_v1;
#[doc(inline)]
pub use __impliterable_props_blank_v1 as impliterable_props_blank_v1;
#[macro_use]
#[path = "macros/props_ccc_v1.rs.data"]
mod props_ccc_v1;
#[doc(inline)]
pub use __impl_props_ccc_v1 as impl_props_ccc_v1;
#[doc(inline)]
pub use __impliterable_props_ccc_v1 as impliterable_props_ccc_v1;
#[macro_use]
#[path = "macros/props_ea_v1.rs.data"]
mod props_ea_v1;
#[doc(inline)]
pub use __impl_props_ea_v1 as impl_props_ea_v1;
#[doc(inline)]
pub use __impliterable_props_ea_v1 as impliterable_props_ea_v1;
#[macro_use]
#[path = "macros/props_exemplarchars_auxiliary_v1.rs.data"]
mod props_exemplarchars_auxiliary_v1;
#[doc(inline)]
pub use __impl_props_exemplarchars_auxiliary_v1 as impl_props_exemplarchars_auxiliary_v1;
#[doc(inline)]
pub use __impliterable_props_exemplarchars_auxiliary_v1 as impliterable_props_exemplarchars_auxiliary_v1;
#[macro_use]
#[path = "macros/props_exemplarchars_index_v1.rs.data"]
mod props_exemplarchars_index_v1;
#[doc(inline)]
pub use __impl_props_exemplarchars_index_v1 as impl_props_exemplarchars_index_v1;
#[doc(inline)]
pub use __impliterable_props_exemplarchars_index_v1 as impliterable_props_exemplarchars_index_v1;
#[macro_use]
#[path = "macros/props_exemplarchars_main_v1.rs.data"]
mod props_exemplarchars_main_v1;
#[doc(inline)]
pub use __impl_props_exemplarchars_main_v1 as impl_props_exemplarchars_main_v1;
#[doc(inline)]
pub use __impliterable_props_exemplarchars_main_v1 as impliterable_props_exemplarchars_main_v1;
#[macro_use]
#[path = "macros/props_exemplarchars_numbers_v1.rs.data"]
mod props_exemplarchars_numbers_v1;
#[doc(inline)]
pub use __impl_props_exemplarchars_numbers_v1 as impl_props_exemplarchars_numbers_v1;
#[doc(inline)]
pub use __impliterable_props_exemplarchars_numbers_v1 as impliterable_props_exemplarchars_numbers_v1;
#[macro_use]
#[path = "macros/props_exemplarchars_punctuation_v1.rs.data"]
mod props_exemplarchars_punctuation_v1;
#[doc(inline)]
pub use __impl_props_exemplarchars_punctuation_v1 as impl_props_exemplarchars_punctuation_v1;
#[doc(inline)]
pub use __impliterable_props_exemplarchars_punctuation_v1 as impliterable_props_exemplarchars_punctuation_v1;
#[macro_use]
#[path = "macros/props_gc_v1.rs.data"]
mod props_gc_v1;
#[doc(inline)]
pub use __impl_props_gc_v1 as impl_props_gc_v1;
#[doc(inline)]
pub use __impliterable_props_gc_v1 as impliterable_props_gc_v1;
#[macro_use]
#[path = "macros/props_graph_v1.rs.data"]
mod props_graph_v1;
#[doc(inline)]
pub use __impl_props_graph_v1 as impl_props_graph_v1;
#[doc(inline)]
pub use __impliterable_props_graph_v1 as impliterable_props_graph_v1;
#[macro_use]
#[path = "macros/props_hst_v1.rs.data"]
mod props_hst_v1;
#[doc(inline)]
pub use __impl_props_hst_v1 as impl_props_hst_v1;
#[doc(inline)]
pub use __impliterable_props_hst_v1 as impliterable_props_hst_v1;
#[macro_use]
#[path = "macros/props_jt_v1.rs.data"]
mod props_jt_v1;
#[doc(inline)]
pub use __impl_props_jt_v1 as impl_props_jt_v1;
#[doc(inline)]
pub use __impliterable_props_jt_v1 as impliterable_props_jt_v1;
#[macro_use]
#[path = "macros/props_lb_v1.rs.data"]
mod props_lb_v1;
#[doc(inline)]
pub use __impl_props_lb_v1 as impl_props_lb_v1;
#[doc(inline)]
pub use __impliterable_props_lb_v1 as impliterable_props_lb_v1;
#[macro_use]
#[path = "macros/props_nfcinert_v1.rs.data"]
mod props_nfcinert_v1;
#[doc(inline)]
pub use __impl_props_nfcinert_v1 as impl_props_nfcinert_v1;
#[doc(inline)]
pub use __impliterable_props_nfcinert_v1 as impliterable_props_nfcinert_v1;
#[macro_use]
#[path = "macros/props_nfdinert_v1.rs.data"]
mod props_nfdinert_v1;
#[doc(inline)]
pub use __impl_props_nfdinert_v1 as impl_props_nfdinert_v1;
#[doc(inline)]
pub use __impliterable_props_nfdinert_v1 as impliterable_props_nfdinert_v1;
#[macro_use]
#[path = "macros/props_nfkcinert_v1.rs.data"]
mod props_nfkcinert_v1;
#[doc(inline)]
pub use __impl_props_nfkcinert_v1 as impl_props_nfkcinert_v1;
#[doc(inline)]
pub use __impliterable_props_nfkcinert_v1 as impliterable_props_nfkcinert_v1;
#[macro_use]
#[path = "macros/props_nfkdinert_v1.rs.data"]
mod props_nfkdinert_v1;
#[doc(inline)]
pub use __impl_props_nfkdinert_v1 as impl_props_nfkdinert_v1;
#[doc(inline)]
pub use __impliterable_props_nfkdinert_v1 as impliterable_props_nfkdinert_v1;
#[macro_use]
#[path = "macros/props_print_v1.rs.data"]
mod props_print_v1;
#[doc(inline)]
pub use __impl_props_print_v1 as impl_props_print_v1;
#[doc(inline)]
pub use __impliterable_props_print_v1 as impliterable_props_print_v1;
#[macro_use]
#[path = "macros/props_sc_v1.rs.data"]
mod props_sc_v1;
#[doc(inline)]
pub use __impl_props_sc_v1 as impl_props_sc_v1;
#[doc(inline)]
pub use __impliterable_props_sc_v1 as impliterable_props_sc_v1;
#[macro_use]
#[path = "macros/props_scx_v1.rs.data"]
mod props_scx_v1;
#[doc(inline)]
pub use __impl_props_scx_v1 as impl_props_scx_v1;
#[doc(inline)]
pub use __impliterable_props_scx_v1 as impliterable_props_scx_v1;
#[macro_use]
#[path = "macros/props_segstart_v1.rs.data"]
mod props_segstart_v1;
#[doc(inline)]
pub use __impl_props_segstart_v1 as impl_props_segstart_v1;
#[doc(inline)]
pub use __impliterable_props_segstart_v1 as impliterable_props_segstart_v1;
#[macro_use]
#[path = "macros/props_xdigit_v1.rs.data"]
mod props_xdigit_v1;
#[doc(inline)]
pub use __impl_props_xdigit_v1 as impl_props_xdigit_v1;
#[doc(inline)]
pub use __impliterable_props_xdigit_v1 as impliterable_props_xdigit_v1;
