package net.pengo.data;


import java.io.*;

/** 
 * NewFile.. currently immutable.
 */
public class EmptyData extends Data {
    static final byte[] nada = new byte[0];
    
    public long getLength() {
        return 0;
    }
    
    //FIXME: throw out of bounds thing
    public Data getSelection(long start, long length) {
        return this;
    }
    
    public TransparentData subdata(long start, long length) {
        return new TransparentData(this,0,0);
    }
    
    
    //public Chunk getMultpleSelection(...);    
    
    public String toString() {
        return "empty file";
    }
        
    public String getType() {
        return "empty file";
    }

    public InputStream dataStream() {
        return new ByteArrayInputStream(nada);
    }
    
    public InputStream getDataStream(long offset) {
        return new ByteArrayInputStream(nada);
    }

    public InputStream getDataStream(long start, long length) {
        return new ByteArrayInputStream(nada);
    }
}
