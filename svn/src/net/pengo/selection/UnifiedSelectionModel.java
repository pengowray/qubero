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
 * UnifiedSelectionModel.java
 *
 * Created on 30 October 2002, 20:24
 */

package net.pengo.selection;


import javax.swing.*;

/**
 *
 * a selection model which stores selection information about a list, and allows
 * the list to be treated as a 2D array, or as two separate Row/Column models.
 *
 * @author Peter Halasz
 */


import javax.swing.*;
import javax.swing.event.*;
import java.util.*;

public class UnifiedSelectionModel implements TableColumnModelListener, ListSelectionListener, TableModelListener {
    int width, height;
	
    LongListSelectionModel lsm = new SegmentalLongListSelectionModel();
    //LongListSelectionModel lsm = new DefaultLongListSelectionModel();
    
    //RowSelectionModel rsm;
    
    // keep track of how many in each row/col selected.
    // if 1 or more, then count [entre] row/col as selected.
    //int[] rowSelectionCount;
    //int[] colSelectionCount;
    
    //ListSelectionModel rowSelectionModel;
    //ListSelectionModel colSelectionModel;
    
    boolean rowIsAdjusting = false;
    boolean colIsAdjusting = false;
    ListSelectionEvent lastAdjustingRowChangeEvent;
    ListSelectionEvent lastRowChangeEvent; // non adjusting event
    ListSelectionEvent lastAdjustingColChangeEvent;
    ListSelectionEvent lastColChangeEvent; // non adjusting event
    JTable jtable;
    
    /** Creates a new instance of UnifiedSelectionModel */
    public UnifiedSelectionModel(JTable table) {
        //FIXME: make an alternative constructor that takes row/col selection models ?
        this.width = table.getColumnCount();
        this.height = table.getRowCount();
        this.jtable = table;
        jtable.getColumnModel().addColumnModelListener(this);
        jtable.getSelectionModel().addListSelectionListener(this);
        jtable.getModel().addTableModelListener(this);
    }
    
    // get the actual selection model
    // FIXME: this class really should just extend a selection model.. no, want to be able to change it
    public LongListSelectionModel getSelectionModel() {
        return lsm;
    }

    public void setSelectionModel(LongListSelectionModel lsm) {
        if (this.lsm == lsm)
            return;
            
        this.lsm = lsm;
        //FIXME: trigger something or copy contents of lsm out or something!
        //FIXME: copy listeners from old lsm at least
        //FIXME: separate setSelection method (not model)
    }
    
    
    /** Tells listeners that a column was added to the model.  */
    public void columnAdded(TableColumnModelEvent e) {
    }
    
    /** Tells listeners that a column was moved due to a margin change.  */
    public void columnMarginChanged(ChangeEvent e) {
    }
    
    /** Tells listeners that a column was repositioned.  */
    public void columnMoved(TableColumnModelEvent e) {
    }
    
    /** Tells listeners that a column was removed from the model.  */
    public void columnRemoved(TableColumnModelEvent e) {
        
    }
    
    /**
     * column change
     */
    public void columnSelectionChanged(ListSelectionEvent e) {
        if (e.getValueIsAdjusting()) {
            //lastAdjustingColChangeEvent = e;
            colIsAdjusting = true;
            lsm.setValueIsAdjusting(true);
        } else {
            if (colIsAdjusting && !rowIsAdjusting) {
                // just changed into non-adjudting state
                lsm.setValueIsAdjusting(false);
            }
            colIsAdjusting = false;
        }
        //setSelection();
    }
    
    /**
     * row change
     */
    public void valueChanged(ListSelectionEvent e) {
        if (e.getValueIsAdjusting()) {
            //lastAdjustingRowChangeEvent = e;
            rowIsAdjusting = true;
            lsm.setValueIsAdjusting(true);
        } else {
            if (rowIsAdjusting && !colIsAdjusting) {
                // just changed into non-adjudting state
                lsm.setValueIsAdjusting(false);
                
            }
            rowIsAdjusting = false;
        }
        setSelection();
    }
    
    /**
     * This fine grain notification tells listeners the exact range
     * of cells, rows, or columns that changed.
     * FIXME: may not be neccesary to get this notification
     */
    public void tableChanged(TableModelEvent e) {
        System.out.println("table cha: " + e);
    }
    
    protected void setSelection() {
	/*
        boolean noAnchor = false; // if true, don't use anchor.
        boolean noLead = false;
        
        int rowAnchor = jtable.getSelectionModel().getAnchorSelectionIndex();
        int rowLead = jtable.getSelectionModel().getLeadSelectionIndex();
        int colAnchor = jtable.getColumnModel().getSelectionModel().getAnchorSelectionIndex();
        int colLead = jtable.getColumnModel().getSelectionModel().getLeadSelectionIndex();
	
        noAnchor = (rowAnchor == -1 || colAnchor == -1);
        noLead = (rowLead == -1 || colLead == -1);
        if (noLead && noAnchor) {
            return;
        } else if (noLead) {
	    System.out.println("setting anchor");
            lsm.setAnchorSelectionIndex(rowAnchor * width + colAnchor);
        } else if (noAnchor) {
	    System.out.println("setting lead");
            lsm.setLeadSelectionIndex(rowLead * width + colLead);
        } else {
	    System.out.println("setting selection");
            //lsm.setSelectionInterval( rowAnchor * width + colAnchor, rowLead * width + colLead );
            lsm.addSelectionInterval( rowAnchor * width + colAnchor, rowLead * width + colLead );
        }
	 */
    }
    
    //additional method to be called by HexTable (or any extended JTable)
    public boolean isCellSelected(int row, int column) {
        return lsm.isSelectedIndex(index(row, column));
    }
	

    public void setAnchorSelectionIndex(int rowIndex, int columnIndex) {
	lsm.setAnchorSelectionIndex(index(rowIndex, columnIndex));
	
	// just to trigger events for redrawing of jtable
	jtable.getSelectionModel().setAnchorSelectionIndex(rowIndex);
        jtable.getColumnModel().getSelectionModel().setAnchorSelectionIndex(columnIndex);
    }

    public void setLeadSelectionIndex(int rowIndex, int columnIndex) {
	lsm.setLeadSelectionIndex(index(rowIndex, columnIndex));

	// just to trigger events for redrawing of jtable
	jtable.getSelectionModel().setLeadSelectionIndex(rowIndex);
        jtable.getColumnModel().getSelectionModel().setLeadSelectionIndex(columnIndex);
    }
    
    public void removeSelectionInterval(int rowIndex0, int columnIndex0, int rowIndex1, int columnIndex1) {
	lsm.removeSelectionInterval(index(rowIndex0, columnIndex0), index(rowIndex1, columnIndex1));

	// just to trigger events for redrawing of jtable
	jtable.getSelectionModel().removeSelectionInterval(rowIndex0, rowIndex1);
        jtable.getColumnModel().getSelectionModel().removeSelectionInterval(columnIndex0, columnIndex1);
    }
    
    public void addSelectionInterval(int rowIndex0, int columnIndex0, int rowIndex1, int columnIndex1) {
	lsm.addSelectionInterval(index(rowIndex0, columnIndex0), index(rowIndex1, columnIndex1));

	// just to trigger events for redrawing of jtable
	jtable.getSelectionModel().addSelectionInterval(rowIndex0, rowIndex1);
        jtable.getColumnModel().getSelectionModel().addSelectionInterval(columnIndex0, columnIndex1);
    }

    public void setSelectionInterval(int rowIndex0, int columnIndex0, int rowIndex1, int columnIndex1) {
	lsm.setSelectionInterval(index(rowIndex0, columnIndex0), index(rowIndex1, columnIndex1));

	// just to trigger events for redrawing of jtable
	jtable.getSelectionModel().setSelectionInterval(rowIndex0, rowIndex1);
        jtable.getColumnModel().getSelectionModel().setSelectionInterval(columnIndex0, columnIndex1);
    }

    protected long index(int row, int column) {
	if (column > 0) {
           return row * width + (column - 1);
	} else {
	    return -2; //FIXME: not good?
	}
    }
}
