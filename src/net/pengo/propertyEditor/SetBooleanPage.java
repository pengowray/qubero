/*
 * SetBooleanPage.java
 *
 * Created on August 24, 2003, 7:03 AM
 */

package net.pengo.propertyEditor;

import javax.swing.JComboBox;
import javax.swing.JLabel;

import net.pengo.resource.BooleanResource;

/**
 *
 * @author  Peter Halasz
 */
public class SetBooleanPage extends EditablePage {
    private BooleanResource boolRes;
    private JComboBox type;
    
    /** Creates a new instance of SetBooleanPage */
    public SetBooleanPage(BooleanResource boolRes, AbstractResourcePropertiesForm form) {
        super(form);
        this.boolRes = boolRes;

        add(new JLabel( "Boolean value: " ));
        type = new JComboBox(new String[] {
            "false (or 0)", "true (or 1)"} );
        add(type);
        build(); // build before listener to stop apply being hit
        type.addActionListener( getModActionListener() );
    }
        
    public void saveOp() {
        boolRes.setValue( (type.getSelectedIndex()==1 ? true:false) );
    }
    
    public void buildOp() {
	int selection = (boolRes.getValue() ? 1:0);
	type.setSelectedIndex( selection  );
    }
    
    public String toString() {
        return "Boolean value";
    }
    
}
