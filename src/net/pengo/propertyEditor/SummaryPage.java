/*
 * SummaryPage.java
 *
 * Created on July 9, 2003, 11:26 PM
 */

package net.pengo.propertyEditor;

import javax.swing.JTextPane;

import net.pengo.resource.IntAddressedResource;

/**
 *
 * @author  Smiley
 */
public class SummaryPage extends PropertyPage {
    private IntAddressedResource res;
    
    /** Creates a new instance of SummaryPage */
    public SummaryPage(IntAddressedResource res) {
        super();
        this.res = res;
        build();
    }
    
    public void build() {
        removeAll();
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        //add(new JLabel( "Size (bytes): " + res.getSelectionData().getLength() ));
        
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        JTextPane jtp = new JTextPane();
        String s = "Value: " + res.getValue() + "\n";
        s = s  + "Size (bytes): " + res.getSelectionResource().getSelectionData().getLength();
        jtp.setText(s ) ;
        add(jtp);
    }
    
    public void save() {
        return;
    }
    
    public String toString() {
        return "Summary";
    }
}
