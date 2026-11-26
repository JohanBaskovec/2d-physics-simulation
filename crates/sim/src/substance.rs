//! The substance table. Every material is a row of data; the engine never
//! refers to a specific substance by name.

use crate::cell::Form;
use std::collections::HashMap;

/// Index into the substance table. 0 means "no substance".
pub type SubId = u16;

/// Properties of one substance in one form, laid out for the hot loops.
#[derive(Clone, Copy, Debug, Default)]
pub struct FormProps {
    /// Molar heat capacity, J/(mol K).
    pub cp: f32,
    /// Enthalpy offset, J/mol. Molar enthalpy is `h0 + cp * T`, measured from
    /// the solid at 0 K, so the offsets of liquid and gas include the latent
    /// heats. Moving moles between forms at constant cell energy therefore
    /// absorbs or releases latent heat with no extra bookkeeping.
    pub h0: f32,
    /// Molar volume, cm^3/mol. Zero for gas, which fills whatever is left.
    pub vm: f32,
    /// Thermal conductivity, W/(m K).
    pub k: f32,
    /// Natural log of `k`, for the geometric mean used in powder cells.
    pub ln_k: f32,
    /// Molar mass, g/mol.
    pub molar_mass: f32,
    /// Density, g/cm^3. Zero for gas.
    pub density: f32,
}

#[derive(Clone, Debug)]
pub struct Substance {
    pub symbol: String,
    pub name: String,
    pub molar_mass: f32,
    pub t_melt: f32,
    pub t_boil: f32,
    pub l_fus: f32,
    pub l_vap: f32,
    pub color: [u8; 3],
}

impl Substance {
    /// The form this substance takes at temperature `t` and 1 atm.
    /// Solids come back as `Rigid`.
    pub fn natural_form(&self, t: f32) -> Form {
        if t < self.t_melt {
            Form::Rigid
        } else if t < self.t_boil {
            Form::Liquid
        } else {
            Form::Gas
        }
    }
}

pub struct Substances {
    list: Vec<Substance>,
    /// `props[id][form]`. Row 0 and column `Form::None` are all zeros, so
    /// empty slots contribute nothing to any sum without a branch.
    props: Vec<[FormProps; 5]>,
    by_symbol: HashMap<String, SubId>,
}

impl Substances {
    /// The table shipped with the engine.
    pub fn builtin() -> Substances {
        Self::from_csv(include_str!("../data/substances.csv")).expect("built-in table is valid")
    }

    pub fn from_csv(text: &str) -> Result<Substances, String> {
        let mut list = vec![Substance {
            symbol: String::new(),
            name: "nothing".into(),
            molar_mass: 0.0,
            t_melt: 0.0,
            t_boil: 0.0,
            l_fus: 0.0,
            l_vap: 0.0,
            color: [0; 3],
        }];
        let mut props = vec![[FormProps::default(); 5]];
        let mut by_symbol = HashMap::new();
        let mut header: Option<Vec<String>> = None;

        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            let Some(cols) = &header else {
                header = Some(fields.iter().map(|s| s.to_string()).collect());
                continue;
            };
            if fields.len() != cols.len() {
                return Err(format!(
                    "line {}: expected {} fields, found {}",
                    line_no + 1,
                    cols.len(),
                    fields.len()
                ));
            }
            let text_of = |name: &str| -> Result<&str, String> {
                cols.iter()
                    .position(|c| c == name)
                    .map(|i| fields[i])
                    .ok_or_else(|| format!("missing column '{name}'"))
            };
            let num = |name: &str| -> Result<f32, String> {
                text_of(name)?
                    .parse::<f32>()
                    .map_err(|e| format!("line {}: column '{name}': {e}", line_no + 1))
            };

            let molar_mass = num("molar_mass")?;
            let (rho_s, rho_l) = (num("rho_solid")?, num("rho_liquid")?);
            let (t_melt, t_boil) = (num("t_melt")?, num("t_boil")?);
            let (cp_s, cp_l, cp_g) = (num("cp_solid")?, num("cp_liquid")?, num("cp_gas")?);
            let (l_fus, l_vap) = (num("l_fus")?, num("l_vap")?);
            let (k_s, k_l, k_g) = (num("k_solid")?, num("k_liquid")?, num("k_gas")?);
            for (name, v) in [
                ("molar_mass", molar_mass),
                ("rho_solid", rho_s),
                ("rho_liquid", rho_l),
                ("cp_solid", cp_s),
                ("cp_liquid", cp_l),
                ("cp_gas", cp_g),
                ("k_solid", k_s),
                ("k_liquid", k_l),
                ("k_gas", k_g),
            ] {
                if !(v > 0.0) {
                    return Err(format!("line {}: '{name}' must be positive", line_no + 1));
                }
            }

            let color_hex = text_of("color")?;
            let rgb = u32::from_str_radix(color_hex, 16)
                .map_err(|e| format!("line {}: color: {e}", line_no + 1))?;
            let color = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];

            // Enthalpy offsets, chosen so enthalpy is continuous except for
            // the latent heat jumps at the melting and boiling points.
            let h0_s = 0.0;
            let h0_l = (cp_s - cp_l) * t_melt + l_fus;
            let h0_g = h0_l + (cp_l - cp_g) * t_boil + l_vap;

            let solid = FormProps {
                cp: cp_s,
                h0: h0_s,
                vm: molar_mass / rho_s,
                k: k_s,
                ln_k: k_s.ln(),
                molar_mass,
                density: rho_s,
            };
            let liquid = FormProps {
                cp: cp_l,
                h0: h0_l,
                vm: molar_mass / rho_l,
                k: k_l,
                ln_k: k_l.ln(),
                molar_mass,
                density: rho_l,
            };
            let gas = FormProps {
                cp: cp_g,
                h0: h0_g,
                vm: 0.0,
                k: k_g,
                ln_k: k_g.ln(),
                molar_mass,
                density: 0.0,
            };
            let mut row = [FormProps::default(); 5];
            row[Form::Rigid as usize] = solid;
            row[Form::Granular as usize] = solid;
            row[Form::Liquid as usize] = liquid;
            row[Form::Gas as usize] = gas;

            let symbol = text_of("symbol")?.to_string();
            if list.len() > SubId::MAX as usize {
                return Err("too many substances".into());
            }
            let id = list.len() as SubId;
            if by_symbol.insert(symbol.clone(), id).is_some() {
                return Err(format!("line {}: duplicate symbol '{symbol}'", line_no + 1));
            }
            list.push(Substance {
                symbol,
                name: text_of("name")?.to_string(),
                molar_mass,
                t_melt,
                t_boil,
                l_fus,
                l_vap,
                color,
            });
            props.push(row);
        }
        if list.len() == 1 {
            return Err("no substances in table".into());
        }
        Ok(Substances { list, props, by_symbol })
    }

    /// Look a substance up by symbol, e.g. `"H2O"`.
    pub fn id(&self, symbol: &str) -> Option<SubId> {
        self.by_symbol.get(symbol).copied()
    }

    /// Like [`id`](Self::id) but panics with a clear message. For setup code.
    pub fn must(&self, symbol: &str) -> SubId {
        self.id(symbol)
            .unwrap_or_else(|| panic!("substance '{symbol}' is not in the table"))
    }

    pub fn get(&self, id: SubId) -> &Substance {
        &self.list[id as usize]
    }

    /// Number of rows, including the "nothing" row at index 0.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.len() <= 1
    }

    /// Ids of all real substances, in table order.
    pub fn ids(&self) -> impl Iterator<Item = SubId> {
        1..self.list.len() as SubId
    }

    #[inline(always)]
    pub fn props(&self, id: SubId, form: Form) -> &FormProps {
        &self.props[id as usize][form as usize]
    }
}
