/*
 * HexTableModel.java
 *
 * Created on 12 September 2002, 13:26
 */
package net.pengo.hexdraw.tabled;

import net.pengo.data.*;

import java.io.*;
import java.util.*;
/**
 *
 * @author  administrator
 */
public class HexTableModel extends javax.swing.table.AbstractTableModel {
    
    
    private int preHexColumns = 1;
    private int hexColumns; // given by constructor
    private int afterHexColumns = 0;
    private int columns = -1; // lazy init
    private int rows;
    private Data data;
    private HexTable parent;
    //private ColumnGroupManager cgm;
    private ArrayList cgm; // ColumnGroupManager
    
    /** Creates a new instance of HexTableModel */
    public HexTableModel(Data data, int hexColumns) {
	super();
	this.hexColumns = hexColumns;
	this.data = data;
	cgm = new ArrayList(hexColumns);
	cgm.add(1).add(2).add(3);
	columns = preHexColumns + hexColumns + afterHexColumns; //FIXME: all columns should be managed by cgm
    }
    
    public int getColumnCount() {
	if (columns != -1)
	    return columns;
	
	int cols = 0;
	for (Iterator i = cgm.iterator(); i.hasNext(); ) {
	    ColumnGroup cg = (ColumnGroup)i.next();
	    cols += cg.getColumnCount();
	}
	
	columns = cols;
	return columns;
    }
    
    /** Creates a new instance of HexTableModel */
    /*
    public HexTableModel(Data data, int hexColumns, ColumnGroupManager cgm) {
        super();
        this.data = data;
        this.hexColumns = hexColumns;
	setColGroupManager(cgm);
        columns = preHexColumns + hexColumns + afterHexColumns;
    }
    
    public void setColGroupManager(ColumnGroupManager cgm){
	this.cgm = cgm;
    }
     */
    

    
    public String getColumnName(int column) {
        if (column < preHexColumns) {
            return "addr";
        } else if (column < preHexColumns + hexColumns) {
            //return "+" + (column - preHexColumns);
            return Integer.toString((column - preHexColumns),16);
        } else {
            return "x";
        }
    }
    
    /** Returns the most specific superclass for all the cell values
     * in the column.  This is used by the <code>JTable</code> to set up a
     * default renderer and editor for the column.
     *
     * @param columnIndex  the index of the column
     * @return the common ancestor class of the object values in the model.
     *
     */
    public Class getColumnClass(int columnIndex) {
        return "".getClass(); //FIXME: for now
    }
    
    public int getRowCount() {
        int ret = (int)(data.getLength() / hexColumns); //FIXME: loss of precision
        if (data.getLength() % hexColumns > 0) {
            // half row
            ret = ret + 1;
        }
        return ret;
    }
    
    public Object getValueAt(int rowIndex, int columnIndex) {
        if (columnIndex < preHexColumns) {
            // address column
            long pos = (rowIndex * hexColumns);
            //return new Long(pos);
            return Long.toString(pos,16);
        } else if (columnIndex >= preHexColumns && columnIndex < preHexColumns+hexColumns) {
            // a hex column
            long pos = (rowIndex * hexColumns) + columnIndex - preHexColumns;
            try {
                InputStream in = data.getDataStream(pos, 1);
                int ret = in.read();
                in.close();
                if (ret == -1) {
                    return null;
                    //return "..";
                }
                //return Integer.toString(ret,16); //FIXME: allows negatives
                return(hexString(ret));
            } catch (IOException e) {
                // swallowed
                return "*";
            }
        } else {
            return "blaaah";
        }
    }
    
    static public String hexString(int b) {
        //FIXME: cache this / lazy init / table of 256 strings?
        
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;

        return ""
            + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
            + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
}
