package net.pengo.data;


import java.io.*;

/** 
 * rudimentary file editing. entire file is kept in an array.
 * this is NOT editableArrayData
 */
public class ArrayData extends Data {
    protected byte[] byteArray;
    String name;
    
    public ArrayData() {
        this.byteArray = new byte[0];
    }

    public ArrayData(byte[] byteArray) {
	this.byteArray = byteArray;
    }	
    
    public ArrayData(byte[] byteArray, String name) {
	this.byteArray = byteArray;
        this.name = name;
    }	

    public InputStream getDataStream(long offset, long length) {
        return new ByteArrayInputStream(byteArray, (int)offset, (int)length); // no loss.
    }

    public InputStream getDataStream(long offset) {
        return new ByteArrayInputStream(byteArray, (int)offset, byteArray.length-(int)offset); // no loss.
    }

    public InputStream dataStream(){
        return new ByteArrayInputStream(byteArray);
    }
    
    public long getLength() {
        return (long)byteArray.length;
    }

    public String toString() {
        if (name == null)
            return "byte array data";
            
        return name;
    }
}
