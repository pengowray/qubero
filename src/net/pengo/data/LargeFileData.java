package net.pengo.data;


import java.io.*;

/** 
 * Data of a "large" file, up to Long.MAX_VALUE in length. (16 million terrabytes)
 * Only viewed part of file is read.
 */
public class LargeFileData extends Data {
    // use FileChannel
    
    protected File file;
    protected String name;
    protected long length;

    public LargeFileData(File file) {
	this.file = file;

        if (!file.isFile()) {
            System.out.println("Not a file. Should probably be throwing an exception really.");
        }
        
        name = file.getName();
        this.length = file.length();
        
        if (length > Integer.MAX_VALUE) {
            System.out.println("Note: Long file being OPENED.");
        }
    }	

    public LargeFileData(String filename) {
        this(new File(filename));
    }

    //FIXME: make long eventually?
    public long getLength() {
        return length;
    }

    public String toString() {
        return name;
    }

    public String getType() {
        return "file";
    }
    
    public InputStream dataStream() {
        try {
            return new FileInputStream(file);
        } catch (FileNotFoundException e) {
            e.printStackTrace();
            return null;
        }
        
    }    
    
    public InputStream getDataStream(long offset, long length) {
    	System.out.println("getDataStream (skip)");
    	//new Error("using old getDataStream method").printStackTrace();
        //FIXME: keep alive a FileInputStream. (or bank of them)
    	//FIXME: o/s dependant watch file for changes
        try {
            FileInputStream fis = new FileInputStream(file);
            fis.skip(offset);
            
            return fis;
        } catch (FileNotFoundException e) {
            e.printStackTrace();
            return null;
        } catch (IOException e) {
            e.printStackTrace();
            return null;
        }
    }

    /* should be preferred to getDataStream, i hope */
    public byte[] readByteArray(long start, int length) throws IOException {
    	//System.out.println("readByteArray (seek)");
        try {
        	RandomAccessFile raf = new RandomAccessFile(file, "r");
        	raf.seek(start); // need to subtract getStart() like in parent? dont think so?
        	byte[] b = new byte[length];
        	raf.readFully(b);
        	return b;
            
        } catch (IOException e) {
            e.printStackTrace();
            return new byte[(int)getLength()];
            // XXX throw
        }
    }    
}
