/*
 * UnifiedSelectionModel.java
 *
 * Created on 30 October 2002, 20:24
 */

import javax.swing.*;

/**
 *
 * a selection model which stores selection information about a list, and allows
 * the list to be treated as a 2D array, or as two separate Row/Column models.
 *
 * @author  Noam Chomsky
 */


import javax.swing.*;
import javax.swing.event.*;
import java.util.*;

public class UnifiedSelectionModel implements TableColumnModelListener, ListSelectionListener, TableModelListener {
    int width, height;
    LongListSelectionModel lsm = new DefaultLongListSelectionModel();
    
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
        //xxx: make an alternative constructor that takes row/col selection models ?
        this.width = table.getColumnCount();
        this.height = table.getRowCount();
        this.jtable = table;
        jtable.getColumnModel().addColumnModelListener(this);
        jtable.getSelectionModel().addListSelectionListener(this);
        jtable.getModel().addTableModelListener(this);
    }
    
    // get the actual selection model
    // xxx: this class really should just extend a selection model
    public LongListSelectionModel getSelectionModel() {
        return lsm;
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
        setSelection();
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
     * xxx: may not be neccesary to get this notification
     */
    public void tableChanged(TableModelEvent e) {
        System.out.println("table cha: " + e);
    }
    
    protected void setSelection() {
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
            lsm.setAnchorSelectionIndex(rowAnchor * width + colAnchor);
        } else if (noAnchor) {
            lsm.setLeadSelectionIndex(rowLead * width + colLead);
        } else {
            lsm.setSelectionInterval( rowAnchor * width + colAnchor, rowLead * width + colLead );
        }
    }
    
    //additional method to be called by HexTable (or any extended JTable)
    public boolean isCellSelected(int row, int column) {
        return lsm.isSelectedIndex(row * width + column);
    }
}