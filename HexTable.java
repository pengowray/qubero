/*
 * HexTable.java
 *
 * Created on 12 September 2002, 13:22
 */
import javax.swing.table.*;
import javax.swing.*;
import java.awt.Color;

/**
 *
 * @author  administrator
 */
public class HexTable extends JTable {
    protected OpenFile openFile;
    
    private final static int hexColumns = 16; //xxx: shouldn't be static
    
    private int[] colStart;
    private int[] colBound;
    
    protected UnifiedSelectionModel unifiedSelectionModel;
    
    /** Creates a new instance of HexTable */
    public HexTable(OpenFile openFile) {
        super();
        Data data = openFile.getData();
        
        setFont(FontMetricsCache.singleton().getFont("hex"));
        TableModel tm = new HexTableModel(data,hexColumns);
        setModel(tm);
        setGridColor(Color.WHITE);
        //setRowSelectionAllowed(true); //default
        setColumnSelectionAllowed(true);
        getSelectionModel().setSelectionMode(ListSelectionModel.MULTIPLE_INTERVAL_SELECTION);
        getTableHeader().getColumnModel().getSelectionModel().setSelectionMode(ListSelectionModel.MULTIPLE_INTERVAL_SELECTION);
        //getColumnModel().setColumnMargin(0);
        calcCols();
        unifiedSelectionModel = new UnifiedSelectionModel(this);
        
        //unifiedSelectionModel.getSelectionModel().addLongListSelectionListener(openFile);
        openFile.setSelectionModel(unifiedSelectionModel.getSelectionModel());
    }
    
    /*
    public String  getColumnName(int column) {
        return Integer.toString(column, 16);
    }
     */
    
    protected void calcCols() {
        // xxx: this whole thing needs a re-write.. 
        // XXX: does not support resize
        // xxx: should have n modes: 1. best-size columns (always bisect units); 
        //      2. fixed sized columns, units alignment changed to group columns
        //      3. extra spacing columns, fixed size columns, fixed position units
        //      4. ascii editor simulator
        // 
        int units = hexColumns;
        //int unitWidth = (int)unitPainter.getPreferredSize().getWidth(); //xxx: precision loss. double->int
        int spacingWidth = FontMetricsCache.singleton().getFontMetrics("hex").charWidth('W') * 5;
        int unitWidth =  FontMetricsCache.singleton().getFontMetrics("hex").charWidth('W') * 2;
        //int spacingWidth = unitWidth;
        
        float align = .5f; // .5 == put half of each space to the left side of the unit
        float alignUnit = .5f; // text will be centered on "start" point. 
        
        int leftPad = unitWidth; // how much to pad the left
        int rightPad = unitWidth;

        if (units == 0) {
            colStart = new int[] { leftPad } ;
            colBound = new int[] { 0, leftPad + rightPad };
            return;
        }
        
        colStart = new int[units];
        colBound = new int[units + 1]; // start and end of unit, with spacing
        
        int[] spaceEvery = new int[] {1, 2, 4, 8}; //xxx: use denominators
        //int[] spaceSize = new int[] {0, 0, 0, 0}; // none?
        int[] spaceSize = new int[] {spacingWidth, 0, spacingWidth*2, spacingWidth};
        //int[] spaceSize = new int[] {spacingWidth, 0, spacingWidth, spacingWidth}; // original
        //int[] spaceSize = new int[] {spacingWidth/2, spacingWidth/2, spacingWidth, spacingWidth};
        
        colBound[0] = 0;
        colStart[0] = leftPad;
        
        for (int n=1; n<units; n++) { 
                
            int gap = 0;
            for (int m=0; m<spaceEvery.length; m++) {
                if ((n) % spaceEvery[m] == 0) {
                    gap += spaceSize[m];
                }
            } 
            
            colStart[n] = colStart[n-1] + unitWidth + gap;
            colBound[n] = colStart[n-1] + unitWidth + (int)(gap * align);
            
        }
        colBound[units] = colBound[units-1] + unitWidth + rightPad;
        colBound[units] = colBound[units-1] + unitWidth + rightPad;
        

        // do the setting of widths:
        for (int col = 0; col < hexColumns; col++) {
            int colWidth = colBound[col+1] - colBound[col]; 
            float colAlign = (float)((colStart[col]+(unitWidth*alignUnit)) - colBound[col]) / colWidth;
            //System.out.println(colBound[col] + "-" + colBound[col+1] + " / " + colStart[col] + " = " + colWidth + " " + colAlign);
            TableColumn column = getColumnModel().getColumn(col+1);
            column.setMinWidth(unitWidth + getColumnModel().getColumnMargin());
            column.setPreferredWidth(colWidth);
            //DefaultTableCellRenderer cr = new DefaultTableCellRenderer();
            TableCellRenderer cr = new HexCellRenderer(colAlign,unitWidth);
            //TableCellRenderer cr = new HexCellRenderer(0.75f,unitWidth); // test
            column.setCellRenderer(cr);
        }
    }
    
    //public ListSelectionModel getSelectionModel()
    
    public void changeSelection(int rowIndex, int columnIndex, boolean toggle, boolean extend) {
        super.changeSelection(rowIndex, columnIndex, toggle, extend);
    }
    
    public boolean isCellSelected(int row, int column) {
        if (column == 0) {
           return false;
        } else {
           return unifiedSelectionModel.isCellSelected(row, column);
           //return super.isCellSelected(row, column);
        }
           
    }
    
}
