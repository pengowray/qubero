/*
 * ShiftedData.java
 *
 * Created on 1 September 2002, 14:04
 */
package net.pengo.data;


import java.io.*;
/**
 *
 * @author  administrator
 */
public class ShiftedData extends Data {
    protected Data sourceData;
    protected long start;
    protected long length;
    protected long shift; // amount to shift the data source. may be + or - or 0.
    
    /** shift source. shift is how far to shift the view of the source data. used for deletes */
    public ShiftedData(long start, long length, long shiftSource, Data sourceData ) {
        this.sourceData = sourceData;
        this.start = start;
        this.length = length;
        this.shift = shiftSource;
        check();
    }
    
    /** shift start. keep the source as is, but shift the start of this view. used for inserts. */
    public ShiftedData(Data sourceData, long start, long length, long shiftStart) {
        this(start + shiftStart, length, 0-shiftStart, sourceData);
    }
    
    protected void check() {
        if (length < 0)
            throw new IllegalArgumentException("Negative length");
    }

    /** shift is how far to shift the view of the source data. used for deletes */
    public Data slideSource(long start, long length, long shiftSource) {
        return new ShiftedData(start, length, shiftSource + shift, sourceData);
    }
    
    public Data shiftStart(long start, long length, long shiftStart) {
        return new ShiftedData(start + shiftStart, length, shift - shiftStart, sourceData);
    }
    
    public Data shiftStart(long shiftStart) {         
        return new ShiftedData(start + shiftStart, length, shift - shiftStart, sourceData);
    }    
    
    public Data getSelection(long start, long length) {
        if (start < this.start || start+length > this.start+this.length) {
            throw new IllegalArgumentException("out of bounds in transparent data");
        }
        return new ShiftedData(start, length, shift, sourceData);
    }

    public String toString() {
        if (length == 1) {
            return "[" + start + ",sourceShift:" + shift + "] == [" + (start+shift) + ",startShift:" + (-shift) + "] source: "+ sourceData;
        } else {
            return "[" + start + "-" + (start+length-1) + ",sourceShift:" + shift + "] == [" + (start+shift) + "-" + (start+length-1+shift) + ",startShift:" + (-shift) + "] source: "+ sourceData;
        }
    }    


    public InputStream dataStream() throws IOException {
        return sourceData.getDataStream(start+shift, length); 
    }
    
    public InputStream getDataStream(long offset) throws IOException {
        return sourceData.getDataStream(this.start+offset+shift, getLength() - offset);
    }
    
    public InputStream getDataStream(long start, long len) throws IOException {
        return sourceData.getDataStream(start+shift, len);
    }
    
    public long getStart() {
        return start;
    }
    
    public long getLength() {
        return length;
    }
    
    //FIXME: more efficient getShifted.... will help debug effort
    
    //FIXME: equals() must be overriden
}
