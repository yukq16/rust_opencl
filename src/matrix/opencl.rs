pub fn dot_product(x: &Vec<f64>, y: &Vec<f64>) -> ocl::Result<(f64)> {
    use ocl::{Platform, Device, Context, Queue, Program,Buffer, Kernel};
  use ocl::enums::{DeviceInfo, DeviceInfoResult, KernelWorkGroupInfo, KernelWorkGroupInfoResult};

  let src = r#"
    __kernel void dot_product(
      __global double* x, 
      __global double* y,
      __global double* partial_sums
    ){
      const int LOCAL_ID = get_local_id(0);
      const int GROUP_SIZE = get_local_size(0);
      const int GROUP_ID = get_group_id(0);
      const size_t PRIVATE_SIZE = 4;

      // Copy to private memory
      double tmp[4];

      for(size_t i = 0; i < PRIVATE_SIZE; ++i){
        tmp[i] = x[GROUP_ID + LOCAL_ID + i] * y[GROUP_ID + LOCAL_ID + i];
      } 

      // Sync
      barrier(CLK_LOCAL_MEM_FENCE);
      
      // Calculate inner product
      double tmp1 = 0.0;
      for(size_t i = 0; i < PRIVATE_SIZE; ++i){
        tmp1 += tmp[i];
      }

      partial_sums[ GROUP_ID ] = tmp1;
    }
  "#;

  let vec_size = x.len();
  let f64_size = 8;
  let platform = Platform::default(); //OSの情報とか
  let device = Device::by_idx_wrap(platform, 0).unwrap();

  println!("Device Name:{:?}", device.name());
  println!("Device Vendor:{:?}", device.vendor());

  let context = Context::builder()
    .platform(platform)
    .devices(device.clone())
    .build()?;
  let program = Program::builder()
    .devices(device)
    .src(src)
    .build(&context).unwrap();
  let queue = Queue::new(&context, device, None)?;

  // 並列化の最大次元数 1 -> ベクトル
  let work_dim : u32 = match device.info(DeviceInfo::MaxWorkItemDimensions){
    Ok(DeviceInfoResult::MaxWorkItemDimensions(res)) => res,
    _ => { 
      println!("failed to get DeviceInfoResult::MaxWorkItemDimensions");
      1
    },
  };

  //ワークグループ総数
  let max_work_group_size: usize = match device.info(DeviceInfo::MaxWorkGroupSize){
    Ok(DeviceInfoResult::MaxWorkGroupSize(res)) => res,
    _ => { 
      println!("failed to get DeviceInfoResult::MaxWorkGroupSize");
      1
    },
  };

  // f64 of Compute Unit(演算ユニット数) ＝ Streaming Multiprocessor(SM) 
  let compute_unit_num :u32 = match device.info(DeviceInfo::MaxComputeUnits){
    Ok(DeviceInfoResult::MaxComputeUnits(res)) => res,
    _ => { 
      println!("failed to get DeviceInfoResult::MaxComputeUnits");
      0
    },
  };

  // ローカルメモリ(スクラッチパッドメモリ)の最大容量
  // このサイズを超えたデータをローカルメモリ(__local修飾子)としてカーネルに渡すと
  // エラーが発生する．
  let max_local_memory_size :u32 = match device.info(DeviceInfo::LocalMemSize){
    Ok(DeviceInfoResult::LocalMemSize(res)) => res as u32,
    _ => { 
      println!("failed to get DeviceInfoResult::LocalMemSize");
      0
    },
  };

  // ワークグループあたりのワークアイテム数？
  let max_work_item_size: Vec<usize> = match device.info(DeviceInfo::MaxWorkItemSizes){
    Ok(DeviceInfoResult::MaxWorkItemSizes(res)) => res,
    _ => { 
      println!("failed to get DeviceInfoResult::MaxWorkGroupSize");
      vec![1; 3]
    },
  };

  // ローカルワークアイテムサイズ
  // 1つのワークグループにつき処理されるワーク数の最適値
  let work_item_num = (vec_size as u32 + compute_unit_num-1) / compute_unit_num;
  
  // ローカルメモリーサイズ
  let local_memory_size = match max_local_memory_size > (vec_size * f64_size) as u32{
    true => vec_size*f64_size,
    false => max_local_memory_size as usize,
  }; 

  let local_array_size  = (local_memory_size + f64_size - 1)/ f64_size;

  println!("input_size:{:?}", vec_size);

  println!("work_dim: {:?}", work_dim);
  println!("compute_unit_num: {:?}", compute_unit_num);
  println!("max_work_group_size:{:?}", max_work_group_size);
  println!("max_local_memory_size:{:?}", max_local_memory_size);
  println!("max_work_item_size:{:?}", max_work_item_size);
  println!("work_item_num:{:?}", work_item_num);
  println!("local_memory_size:{:?}", local_memory_size);
  println!("local_array_size:{:?}", local_array_size);

  let source_buffer_x = Buffer::builder()
    .queue(queue.clone())
    .flags(MemFlags::new().read_only())
    .len(vec_size)
    .copy_host_slice(&x)
    .build()?;

  let source_buffer_y = Buffer::builder()
    .queue(queue.clone())
    .flags(MemFlags::new().read_only())
    .len(vec_size)
    .copy_host_slice(&y)
    .build()?;

  let work_group_size = 1;
  let private_size = 4;
  let work_group_num = vec_size / private_size;
  println!("work_group_size:{:?}", work_group_size);
  println!("work_group_num:{:?}", work_group_num);

  let output_buffer = Buffer::<f64>::builder()
    .queue(queue.clone())
    .flags(MemFlags::new().write_only())
    .len(work_group_num)
    .build()?;

  let kernel : ocl::Kernel = Kernel::builder()
    .program(&program)
    .name("dot_product")
    .queue(queue.clone())
    .arg(&source_buffer_x) 
    .arg(&source_buffer_y)
    .arg(&output_buffer)
    // .arg(&private_size)
    .build().unwrap();


  let preferred_work_group_size_multiple: usize = match kernel.wg_info(device, KernelWorkGroupInfo::PreferredWorkGroupSizeMultiple){
    Ok(KernelWorkGroupInfoResult::PreferredWorkGroupSizeMultiple(res)) => res,
    _ => { 
      println!("failed to get DeviceInfoResult::PreferredWorkGroupSizeMultiple");
      1
    },
  };

  println!("PreferredWorkGroupSizeMultiple:{:?}", preferred_work_group_size_multiple);

  let res: ocl::Result<()>;
  unsafe { 
     res = kernel.cmd()
      .queue(&queue)
      .global_work_offset(kernel.default_global_work_offset())
      .global_work_size(work_group_num)
      .local_work_size(work_group_size)
      .enq();
  }

  match res {
    Ok(_) => {
      println!("success execute kernel code");
      let mut result = vec![0f64; work_group_num];
      output_buffer.cmd()
        .queue(&queue)
        .offset(0)
        .read(&mut result)
        .enq()?;
      Ok(result)
    },
    Err(err) => {
      println!("{:?}", err);
      Err(err)
    },
  }
}
