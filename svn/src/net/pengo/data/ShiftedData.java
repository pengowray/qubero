/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/*
 * ShiftedData.java
 *
 * Created on 1 September 2002, 14:04
 */
package net.pengo.data;


import java.io.*;
/**
 *
 * @author  Peter Halasz
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
