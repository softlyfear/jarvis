$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JarvisAudioMute {
    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] private class Enumerator {}
    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDevices {
        void EnumAudioEndpoints(int flow, uint mask, out IntPtr devices);
        void GetDefaultAudioEndpoint(int flow, int role, out IDevice device);
    }
    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDevice {
        void Activate(ref Guid id, uint context, IntPtr parameters, [MarshalAs(UnmanagedType.IUnknown)] out object result);
    }
    [ComImport, Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IVolume {
        void RegisterControlChangeNotify(IntPtr callback);
        void UnregisterControlChangeNotify(IntPtr callback);
        void GetChannelCount(out uint count);
        void SetMasterVolumeLevel(float level, IntPtr context);
        void SetMasterVolumeLevelScalar(float level, IntPtr context);
        void GetMasterVolumeLevel(out float level);
        void GetMasterVolumeLevelScalar(out float level);
        void SetChannelVolumeLevel(uint channel, float level, IntPtr context);
        void SetChannelVolumeLevelScalar(uint channel, float level, IntPtr context);
        void GetChannelVolumeLevel(uint channel, out float level);
        void GetChannelVolumeLevelScalar(uint channel, out float level);
        void SetMute([MarshalAs(UnmanagedType.Bool)] bool muted, IntPtr context);
        void GetMute([MarshalAs(UnmanagedType.Bool)] out bool muted);
    }
    public static bool Set(bool muted) {
        IDevices devices = null; IDevice device = null; object endpoint = null;
        try {
            devices = (IDevices)new Enumerator();
            devices.GetDefaultAudioEndpoint(0, 1, out device);
            Guid id = typeof(IVolume).GUID;
            device.Activate(ref id, 23, IntPtr.Zero, out endpoint);
            IVolume volume = (IVolume)endpoint;
            volume.SetMute(muted, IntPtr.Zero);
            bool actual; volume.GetMute(out actual);
            if (actual != muted) throw new InvalidOperationException("Mute state was not applied");
            return actual;
        } finally {
            if (endpoint != null) Marshal.ReleaseComObject(endpoint);
            if (device != null) Marshal.ReleaseComObject(device);
            if (devices != null) Marshal.ReleaseComObject(devices);
        }
    }
}
'@
[JarvisAudioMute]::Set($env:JARVIS_MUTED -eq 'true')
