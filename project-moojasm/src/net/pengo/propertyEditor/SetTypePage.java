/*
 * SummaryPage.java
 *
 * Created on July 9, 2003, 11:26 PM
 */

package net.pengo.propertyEditor;
import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.awt.event.*;

/**
 *
 * @author  Smiley
 */
public class SetTypePage extends EditablePage {
    private IntAddressedResource  res;
    private JComboBox type;
    
    /** Creates a new instance of SummaryPage */
    public SetTypePage(IntAddressedResource res, AbstractResourcePropertiesForm form) {
        super(form);
        this.res = res;

        add(new JLabel( "Integer Type: " ));
        type = new JComboBox(new String[] {
            "unsigned", "ones complement (+/-0)", "twos complement (default)", "sign magnitude", "unused sign bit (NYI)"} );
        add(type);
        type.addActionListener( getModActionListener() );
        
        build();
    }
    
    public void buildOp() {
        type.setSelectedIndex( res.getSigned() );
    }
    
    public void saveOp() {
        res.setSigned( type.getSelectedIndex() );
        System.out.println("setting sign to: " + type.getSelectedIndex());
    }
    
    public String toString() {
        return "Type";
    }
}
