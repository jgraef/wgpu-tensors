use std::marker::PhantomData;

use super::{MapKernel, MapSignature};
use crate::{
    element::{
        Element,
        Number, Encode,
    },
    error::KernelError,
    kernel::{
        binding::{
            KernelBindingBuilder,
            KernelBindingDeclaration,
            KernelDeclaration,
            KernelParameterDeclaration,
        },
        map::Map,
        KernelSignature,
        TaskPartition,
    },
    tensor::{
        strider::contiguous_strides,
        Tensor,
    },
};

#[derive(Debug)]
pub struct UnaryArgs<'a, const D: usize, R: Element, A: Element> {
    pub result: &'a mut Tensor<D, R>,
    pub operand: &'a Tensor<D, A>,
}

pub struct UnarySignature<R: Element, A: Element>(PhantomData<(R, A)>);

impl<R: Element, A: Element> KernelSignature for UnarySignature<R, A> {
    const DECLARATION: KernelDeclaration = KernelDeclaration {
        bindings: &[
            KernelBindingDeclaration::read_write::<R>("result"),
            KernelBindingDeclaration::read_only::<A>("operand"),
        ],
        parameters: &[
            KernelParameterDeclaration::shaped("op_strides"),
            KernelParameterDeclaration::shaped("op_shape"),
            KernelParameterDeclaration::int("result_offset"),
            KernelParameterDeclaration::shaped("result_strides"),
            KernelParameterDeclaration::int("operand_offset"),
            KernelParameterDeclaration::shaped("operand_strides"),
        ],
    };

    type Args<'a, const D: usize> = UnaryArgs<'a, D, R, A>;

    fn build_bind_group<'gpu, 'tensor, const D: usize>(
        args: Self::Args<'tensor, D>,
        builder: &mut KernelBindingBuilder<'gpu, 'tensor, D>,
    ) -> Result<(), KernelError> {
        builder.add_binding("result", args.result)?;
        builder.add_binding("operand", args.operand)?;

        let result_strider = args.result.strider();
        let op_shape = result_strider.shape();
        builder.add_parameter("op_strides", contiguous_strides(&op_shape))?;
        builder.add_parameter("op_shape", op_shape)?;

        builder.add_parameter("result_offset", result_strider.offset())?;
        builder.add_parameter("result_strides", result_strider.strides())?;

        let operand_strider = args.operand.strider();
        builder.add_parameter("operand_offset", operand_strider.offset())?;
        builder.add_parameter("operand_strides", operand_strider.strides())?;

        Ok(())
    }

    fn task_partition<'a, const D: usize>(args: &Self::Args<'a, D>) -> TaskPartition {
        TaskPartition::for_result(&args.result)
    }
}

impl<R: Element, A: Element> MapSignature for UnarySignature<R, A> {
    const INPUTS: &'static [&'static str] = &["operand"];
    const OUTPUTS: &'static [&'static str] = &["result"];
}

impl<const D: usize, T: Element> Tensor<D, T> {
    pub async fn map_unary_elementwise<'a, M: Map<Signature = UnarySignature<R, T>>, R: Element>(
        &self,
    ) -> Result<Tensor<D, R>, KernelError> {
        let mut result = Tensor::allocate(&self.gpu, self.shape());
        self.gpu
            .run_kernel::<D, MapKernel<M>>(UnaryArgs {
                result: &mut result,
                operand: self,
            })
            .await?;
        Ok(result)
    }
}

pub struct Identity<T>(PhantomData<T>);
impl<T: Element> Map for Identity<T> {
    const LABEL: &'static str = "Identity";
    const BODY: &'static str = "let value_result = value_operand;";
    type Signature = UnarySignature<T, T>;
}

impl<const D: usize, T: Element> Tensor<D, T> {
    pub async fn id(&self) -> Result<Tensor<D, T>, KernelError> {
        self.map_unary_elementwise::<Identity<T>, T>().await
    }
}

pub struct ElementwiseNegate<T>(PhantomData<T>);
impl<T: Element + Number> Map for ElementwiseNegate<T> {
    const LABEL: &'static str = "ElementwiseNegate";
    const BODY: &'static str = "let value_result = -value_operand;";
    type Signature = UnarySignature<T, T>;
}

impl<const D: usize, T: Element + Number> Tensor<D, T> {
    pub async fn neg(&self) -> Result<Tensor<D, T>, KernelError> {
        self.map_unary_elementwise::<ElementwiseNegate<T>, T>()
            .await
    }
}

pub enum ElementwiseBoolNot {}
impl Map for ElementwiseBoolNot {
    const LABEL: &'static str = "ElementwiseBoolNot";
    const BODY: &'static str = "let value_result = ~value_operand;";
    type Signature = UnarySignature<bool, bool>;
    const INDEX_STEP: usize = <bool as Encode>::NUM_PACKED;
    const MAP_ENCODED: bool = true;
}

impl<const D: usize> Tensor<D, bool> {
    pub async fn not(&self) -> Result<Tensor<D, bool>, KernelError> {
        self.map_unary_elementwise::<ElementwiseBoolNot, bool>()
            .await
    }
}

macro_rules! unary_func_kernel {
    ($kernel:ident, $wsgl_func:ident) => {
        pub struct $kernel<T>(PhantomData<T>);

        impl<T: Element + Number> Map for $kernel<T> {
            const LABEL: &'static str = stringify!($kernel);
            const BODY: &'static str = concat!(
                "let value_result = ",
                stringify!($wsgl_func),
                "(value_operand);"
            );
            type Signature = UnarySignature<T, T>;
        }
    };
}

macro_rules! unary_tensor_impl {
    ($kernel:ident, $tensor_func:ident) => {
        impl<const D: usize, T: Element + Number> Tensor<D, T> {
            pub async fn $tensor_func(&self) -> Result<Tensor<D, T>, KernelError> {
                self.map_unary_elementwise::<$kernel<T>, T>().await
            }
        }
    };
}

macro_rules! unary_func {
    ($kernel:ident, $wsgl_func:ident, $tensor_func:ident) => {
        unary_func_kernel!($kernel, $wsgl_func);
        unary_tensor_impl!($kernel, $tensor_func);
    };
    ($kernel:ident, $func:ident) => {
        unary_func!($kernel, $func, $func);
    };
}

unary_func!(ElementwiseDegrees, degrees);
unary_func!(ElementwiseRadians, radians);
unary_func!(ElementwiseCos, cos);
unary_func!(ElementwiseCosh, cosh);
unary_func!(ElementwiseAcos, acos);
unary_func!(ElementwiseAcosh, acosh);
unary_func!(ElementwiseSin, sin);
unary_func!(ElementwiseSinh, sinh);
unary_func!(ElementwiseAsin, asin);
unary_func!(ElementwiseAsinh, asinh);
unary_func!(ElementwiseTan, tan);
unary_func!(ElementwiseTanh, tanh);
unary_func!(ElementwiseAtan, atan);
unary_func!(ElementwiseAtanh, atanh);
unary_func!(ElementwiseAtan2, atan2);
unary_func!(ElementwiseExp, exp);
unary_func!(ElementwiseExp2, exp2);
unary_func!(ElementwiseLog, log);
unary_func!(ElementwiseLog2, log2);
unary_func!(ElementwiseSqrt, sqrt);
unary_func!(ElementwiseInverseSqrt, inverseSqrt, inverse_sqrt);
unary_func!(ElementwiseAbsolute, abs);
unary_func!(ElementwiseSignum, sign);
unary_func!(ElementwiseFractional, fract);
unary_func!(ElementwiseTruncate, trunc);
unary_func!(ElementwiseCeil, ceil);
unary_func!(ElementwiseFloor, floor);
unary_func!(ElementwiseRound, round);
unary_func!(ElementwiseSaturate, saturate);
